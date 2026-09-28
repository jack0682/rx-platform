//! Skill composition uses the existing ProcessSource structure and validator.
//! This LOCAL_SIM execution profile currently admits Sequence/Operation only.
//! It never converts a software result into device qualification or handover.
use super::*;
use rx_domain::operation::{Integrity, Knowledge};
use rx_process_contract::{NodeBody, ProcessSource, source_validation};
use std::collections::BTreeSet;

const PROCESS: &str = "rx.local-sim.process.v1";
const PROCESS_RUN: &str = "rx.local-sim.process-run.v1";
const RESERVATION: &str = "rx.local-sim.process-child.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum ValueRef {
    Input { field: String },
    Output { step: Name, field: String },
    Literal { value: Value },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub skill: String,
    pub version: String,
    pub inputs: BTreeMap<String, ValueRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub source: ProcessSource,
    pub version: String,
    pub environment: String,
    pub inputs: BTreeMap<String, String>,
    pub bindings: BTreeMap<Name, Binding>,
    pub outputs: BTreeMap<String, ValueRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registered {
    pub digest: Digest,
    pub definition: Definition,
    pub steps: Vec<Name>,
    pub skill_digests: BTreeMap<Name, Digest>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartProcess {
    pub request_id: Id,
    pub process: String,
    pub version: String,
    pub input: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub process_run: Id,
    pub step: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRun {
    request: StartProcess,
    definition_digest: Digest,
    submitted_ms: u64,
    children: BTreeMap<Name, Id>,
    issue: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Queued,
    Active,
    Succeeded,
    Failed,
    Unknown,
    Blocked,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepView {
    pub step: Name,
    pub skill: String,
    pub version: String,
    pub package_digest: Digest,
    pub request_id: Id,
    pub execution: Option<Run>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub request: StartProcess,
    pub definition_digest: Digest,
    pub submitted_ms: u64,
    pub finished_ms: Option<u64>,
    pub status: Status,
    pub steps: Vec<StepView>,
    pub output: Option<Value>,
    pub issue: Option<String>,
}
fn process_key(name: &str, ver: &str) -> Result<Name> {
    Name::new(name).map_err(invalid)?;
    if name.len() > 64 || !version(ver) {
        return Err(invalid("process name or version invalid"));
    }
    key(&format!("local-sim/process/{name}/{ver}"))
}
fn request_key(id: &Id) -> Result<Name> {
    key(&format!("local-sim/process-run/{id}"))
}
fn reserved_key(id: &Id) -> Result<Name> {
    key(&format!("local-sim/reserved/{id}"))
}

pub(super) fn allocated_runs(tx: &mut dyn Transaction) -> Result<usize> {
    let standalone = tx
        .scan("local-sim/run/")?
        .iter()
        .map(|r| decode::<Run>(r, RUN))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .filter(|r| r.parent.is_none())
        .count();
    Ok(standalone + tx.scan("local-sim/reserved/")?.len())
}
pub(super) fn authorize_child(
    tx: &mut dyn Transaction,
    id: &Id,
    parent: Option<&Parent>,
) -> Result<()> {
    if tx.get(&request_key(id)?)?.is_some() {
        return Err(StoreError::KeyConflict);
    }
    let reserved = tx.get(&reserved_key(id)?)?;
    match (reserved, parent) {
        (None, None) => Ok(()),
        (Some(row), Some(parent)) if decode::<Parent>(&row, RESERVATION)? == *parent => Ok(()),
        _ => Err(StoreError::KeyConflict),
    }
}
fn steps(source: &ProcessSource) -> Result<Vec<Name>> {
    let report = source_validation::validate(source);
    if !report.structurally_valid {
        return Err(invalid(format!(
            "invalid process source: {}",
            serde_json::to_string(&report.issues).map_err(invalid)?
        )));
    }
    if source.flows.len() != 1 || !source.conditions.is_empty() {
        return Err(invalid(
            "skill process draft supports one condition-free sequence flow",
        ));
    }
    let flow = &source.flows[0];
    let nodes: BTreeMap<_, _> = flow.nodes.iter().map(|n| (&n.id, &n.body)).collect();
    fn walk(id: &Name, nodes: &BTreeMap<&Name, &NodeBody>, out: &mut Vec<Name>) -> Result<()> {
        match nodes
            .get(id)
            .ok_or_else(|| invalid("missing process node"))?
        {
            NodeBody::Sequence { children } => {
                for child in children {
                    walk(child, nodes, out)?;
                }
            }
            NodeBody::Operation { binding } => out.push(binding.clone()),
            _ => {
                return Err(invalid(
                    "unsupported node in serial skill profile; no control semantics are flattened",
                ));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(&flow.root, &nodes, &mut out)?;
    if out.is_empty() || out.len() > 64 || out.iter().collect::<BTreeSet<_>>().len() != out.len() {
        return Err(invalid(
            "require 1..64 operations with distinct binding names",
        ));
    }
    Ok(out)
}
fn field_type<'a>(
    reference: &'a ValueRef,
    inputs: &'a BTreeMap<String, String>,
    prior: &'a BTreeMap<Name, Package>,
) -> Result<Option<&'a str>> {
    let kind = match reference {
        ValueRef::Input { field } => inputs
            .get(field)
            .ok_or_else(|| invalid(format!("unknown process input: {field}")))?,
        ValueRef::Output { step, field } => prior
            .get(step)
            .and_then(|p| p.outputs.get(field))
            .ok_or_else(|| {
                invalid(format!(
                    "missing or forward output reference: {step}/{field}"
                ))
            })?,
        ValueRef::Literal { .. } => return Ok(None),
    };
    Ok(Some(kind))
}
fn validate_reference(
    reference: &ValueRef,
    expected: &str,
    inputs: &BTreeMap<String, String>,
    prior: &BTreeMap<Name, Package>,
) -> Result<()> {
    if let Some(actual) = field_type(reference, inputs, prior)? {
        if actual != expected && !(actual == "integer" && expected == "number") {
            return Err(invalid("process port type mismatch"));
        }
    } else if let ValueRef::Literal { value } = reference {
        shape(
            &[("value".to_owned(), expected.to_owned())].into(),
            &json!({"value":value}),
        )?;
    }
    Ok(())
}
fn resolve(reference: &ValueRef, input: &Value, completed: &BTreeMap<Name, Run>) -> Result<Value> {
    match reference {
        ValueRef::Input { field } => input
            .get(field)
            .cloned()
            .ok_or_else(|| invalid("process input absent")),
        ValueRef::Output { step, field } => completed
            .get(step)
            .filter(|r| passed(r))
            .and_then(|r| r.output.as_ref())
            .and_then(|v| v.get(field))
            .cloned()
            .ok_or_else(|| invalid("output is not from a validated successful predecessor")),
        ValueRef::Literal { value } => Ok(value.clone()),
    }
}
fn passed(run: &Run) -> bool {
    run.operation.outcome() == Outcome::Succeeded && run.operation.integrity() == Integrity::Valid
}
fn registered(tx: &mut dyn Transaction, run: &StoredRun) -> Result<Registered> {
    let row = tx
        .get(&process_key(&run.request.process, &run.request.version)?)?
        .ok_or_else(|| invalid("process version missing"))?;
    let definition: Registered = decode(&row, PROCESS)?;
    if definition.digest != run.definition_digest {
        return Err(StoreError::Integrity("process definition changed".into()));
    }
    Ok(definition)
}
fn view(tx: &mut dyn Transaction, run: StoredRun) -> Result<View> {
    let definition = registered(tx, &run)?;
    let mut status = Status::Succeeded;
    let mut complete = BTreeMap::new();
    let mut items = vec![];
    let mut stop = false;
    let mut finished = None;
    for step in &definition.steps {
        let binding = &definition.definition.bindings[step];
        let child = run
            .children
            .get(step)
            .ok_or_else(|| invalid("process child identity missing"))?;
        let execution = tx
            .get(&run_key(child)?)?
            .map(|r| decode::<Run>(&r, RUN))
            .transpose()?;
        if let Some(value) = &execution {
            if stop
                || value.parent.as_ref()
                    != Some(&Parent {
                        process_run: run.request.request_id.clone(),
                        step: step.clone(),
                    })
                || value.request.skill != binding.skill
                || value.request.version != binding.version
                || value.package_digest != definition.skill_digests[step]
            {
                return Err(StoreError::Integrity(
                    "process child ownership/order/content differs".into(),
                ));
            }
            if passed(value) {
                complete.insert(step.clone(), value.clone());
                finished = value.finished_ms;
            } else {
                stop = true;
                status = if value.operation.outcome() == Outcome::Unresolved
                    || value.operation.knowledge() == Knowledge::Unknown
                    || value.operation.integrity() != Integrity::Valid
                {
                    Status::Unknown
                } else if value.operation.phase() == Phase::Settled {
                    Status::Failed
                } else if value.started_ms.is_some() {
                    Status::Active
                } else {
                    Status::Queued
                };
                finished = if matches!(status, Status::Unknown | Status::Failed) {
                    value.finished_ms
                } else {
                    None
                };
            }
        } else if !stop {
            stop = true;
            status = Status::Queued;
            finished = None;
        }
        items.push(StepView {
            step: step.clone(),
            skill: binding.skill.clone(),
            version: binding.version.clone(),
            package_digest: definition.skill_digests[step],
            request_id: child.clone(),
            execution,
        });
    }
    let mut issue = run.issue.clone();
    if issue.is_some() {
        status = Status::Blocked;
    }
    let mut output = if status == Status::Succeeded {
        let fields = definition
            .definition
            .outputs
            .iter()
            .map(|(name, r)| Ok((name.clone(), resolve(r, &run.request.input, &complete)?)))
            .collect::<Result<serde_json::Map<_, _>>>()?;
        Some(Value::Object(fields))
    } else {
        None
    };
    if output
        .as_ref()
        .is_some_and(|v| canonical::bytes(v).map_or(true, |b| b.len() > 65_536))
    {
        status = Status::Blocked;
        output = None;
        issue = Some("process output exceeds 64 KiB".into());
    }
    Ok(View {
        request: run.request,
        definition_digest: run.definition_digest,
        submitted_ms: run.submitted_ms,
        finished_ms: finished,
        status,
        steps: items,
        output,
        issue,
    })
}
pub(super) fn advance(tx: &mut dyn Transaction, now: u64) -> Result<()> {
    for row in tx.scan("local-sim/process-run/")? {
        let mut run: StoredRun = decode(&row, PROCESS_RUN)?;
        let current = view(tx, run.clone())?;
        if current.status != Status::Queued {
            continue;
        }
        let Some(next) = current.steps.iter().find(|s| s.execution.is_none()) else {
            continue;
        };
        // An admitted predecessor may still be queued; never skip it.
        if current
            .steps
            .iter()
            .take_while(|s| s.step != next.step)
            .any(|s| s.execution.as_ref().is_none_or(|r| !passed(r)))
        {
            continue;
        }
        let definition = registered(tx, &run)?;
        let binding = &definition.definition.bindings[&next.step];
        let completed = current
            .steps
            .iter()
            .filter_map(|s| {
                s.execution
                    .as_ref()
                    .filter(|r| passed(r))
                    .map(|r| (s.step.clone(), r.clone()))
            })
            .collect();
        let resolved = (|| {
            let input = binding
                .inputs
                .iter()
                .map(|(name, r)| Ok((name.clone(), resolve(r, &run.request.input, &completed)?)))
                .collect::<Result<serde_json::Map<_, _>>>()?;
            let source = tx
                .get(&skill_key(&binding.skill, &binding.version)?)?
                .ok_or_else(|| invalid("skill missing"))?;
            let skill: Skill = decode(&source, SKILL)?;
            let input = Value::Object(input);
            shape(&skill.package.inputs, &input)?;
            Ok(input)
        })();
        let input = match resolved {
            Ok(value) => value,
            Err(StoreError::Invalid(detail)) => {
                run.issue = Some(format!("{}: {detail}", next.step));
                tx.put(
                    &request_key(&run.request.request_id)?,
                    Some(row.revision),
                    &document(PROCESS_RUN, &run)?,
                )?;
                tx.append(&uid(),&document("rx.local-sim.process-blocked.v1",&json!({"run":run.request.request_id,"step":next.step,"reason":run.issue,"observed_ms":now}))?)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        admit(
            tx,
            Start {
                request_id: next.request_id.clone(),
                skill: binding.skill.clone(),
                version: binding.version.clone(),
                input,
            },
            now,
            Some(Parent {
                process_run: run.request.request_id.clone(),
                step: next.step.clone(),
            }),
        )?;
    }
    Ok(())
}
impl<R: Repository> Engine<R> {
    pub fn register_process(&mut self, definition: Definition) -> Result<Registered> {
        let pk = process_key(definition.source.process.as_str(), &definition.version)?;
        if definition.environment != "LOCAL_SIM" {
            return Err(invalid("LOCAL_SIM process required"));
        }
        schema(&definition.inputs)?;
        if definition.outputs.len() > 32 || definition.outputs.keys().any(|k| !symbol(k)) {
            return Err(invalid("invalid process outputs"));
        }
        let ordered = steps(&definition.source)?;
        if definition.bindings.keys().cloned().collect::<BTreeSet<_>>()
            != ordered.iter().cloned().collect()
        {
            return Err(invalid("unused or missing skill bindings"));
        }
        if canonical::bytes(&definition).map_err(invalid)?.len() > 262_144 {
            return Err(invalid("process definition limit"));
        }
        self.repository.transact(|tx| {
            let mut previous = BTreeMap::new();
            let mut digests = BTreeMap::new();
            for step in &ordered {
                let b = &definition.bindings[step];
                let row = tx.get(&skill_key(&b.skill, &b.version)?)?.ok_or_else(|| {
                    invalid(format!("skill not registered: {}@{}", b.skill, b.version))
                })?;
                let skill: Skill = decode(&row, SKILL)?;
                if b.inputs.keys().collect::<BTreeSet<_>>() != skill.package.inputs.keys().collect()
                {
                    return Err(invalid("skill input bindings differ"));
                }
                for (field, reference) in &b.inputs {
                    validate_reference(
                        reference,
                        &skill.package.inputs[field],
                        &definition.inputs,
                        &previous,
                    )?;
                }
                digests.insert(step.clone(), skill.digest);
                previous.insert(step.clone(), skill.package);
            }
            for reference in definition.outputs.values() {
                field_type(reference, &definition.inputs, &previous)?;
            }
            let digest = canonical::digest("RX-LOCAL-SIM-PROCESS-v1", &(&definition, &digests))
                .map_err(invalid)?;
            if let Some(row) = tx.get(&pk)? {
                let old: Registered = decode(&row, PROCESS)?;
                if old.digest != digest {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(old);
            }
            if tx.scan("local-sim/process/")?.len() >= 128 {
                return Err(invalid("process version limit"));
            }
            let registered = Registered {
                digest,
                definition,
                steps: ordered,
                skill_digests: digests,
            };
            tx.put(&pk, None, &document(PROCESS, &registered)?)?;
            Ok(registered)
        })
    }
    pub fn processes(&mut self) -> Result<Vec<Registered>> {
        self.repository.transact(|tx| {
            tx.scan("local-sim/process/")?
                .iter()
                .map(|r| decode(r, PROCESS))
                .collect()
        })
    }
    pub fn start_process(&mut self, request: StartProcess, now: u64) -> Result<View> {
        let pk = process_key(&request.process, &request.version)?;
        self.repository.transact(|tx| {
            if let Some(row) = tx.get(&request_key(&request.request_id)?)? {
                let prior: StoredRun = decode(&row, PROCESS_RUN)?;
                if prior.request != request {
                    return Err(StoreError::KeyConflict);
                }
                return view(tx, prior);
            }
            if tx.get(&run_key(&request.request_id)?)?.is_some()
                || tx.get(&reserved_key(&request.request_id)?)?.is_some()
            {
                return Err(StoreError::KeyConflict);
            }
            let row = tx
                .get(&pk)?
                .ok_or_else(|| invalid("process not registered"))?;
            let definition: Registered = decode(&row, PROCESS)?;
            shape(&definition.definition.inputs, &request.input)?;
            if tx.scan("local-sim/process-run/")?.len() >= 128
                || allocated_runs(tx)? + definition.steps.len() > LIMIT
            {
                return Err(invalid("process/run capacity exhausted"));
            }
            let mut children = BTreeMap::new();
            for step in &definition.steps {
                let child = uid();
                if child == request.request_id || tx.get(&run_key(&child)?)?.is_some() {
                    return Err(invalid("child identity collision"));
                }
                tx.put(
                    &reserved_key(&child)?,
                    None,
                    &document(
                        RESERVATION,
                        &Parent {
                            process_run: request.request_id.clone(),
                            step: step.clone(),
                        },
                    )?,
                )?;
                children.insert(step.clone(), child);
            }
            let run = StoredRun {
                request,
                definition_digest: definition.digest,
                submitted_ms: now,
                children,
                issue: None,
            };
            tx.put(
                &request_key(&run.request.request_id)?,
                None,
                &document(PROCESS_RUN, &run)?,
            )?;
            tx.append(&uid(), &document("rx.local-sim.process-admitted.v1", &run)?)?;
            advance(tx, now)?;
            let current = tx
                .get(&request_key(&run.request.request_id)?)?
                .ok_or_else(|| invalid("process run disappeared during admission"))?;
            view(tx, decode(&current, PROCESS_RUN)?)
        })
    }
    pub fn process_run(&mut self, id: &Id) -> Result<View> {
        self.repository.transact(|tx| {
            let row = tx
                .get(&request_key(id)?)?
                .ok_or_else(|| invalid("process run not found"))?;
            view(tx, decode(&row, PROCESS_RUN)?)
        })
    }
    pub fn process_runs(&mut self) -> Result<Vec<View>> {
        self.repository.transact(|tx| {
            tx.scan("local-sim/process-run/")?
                .into_iter()
                .map(|r| view(tx, decode(&r, PROCESS_RUN)?))
                .collect()
        })
    }
}
