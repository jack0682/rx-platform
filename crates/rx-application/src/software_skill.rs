//! Local simulation skills. This application does not grant Cell/Host authority.
//! Uses the existing repository transaction and Operation state model.
use rx_domain::{
    canonical,
    operation::{Conclusion, Operation, Outcome, Phase},
    types::*,
};
use rx_ports::{Document, Record, Repository, Result, StoreError, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub mod metrics;
pub mod process;

const SKILL: &str = "rx.local-sim.skill.v1";
const RUN: &str = "rx.local-sim.run.v1";
const LIMIT: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub environment: String,
    pub inputs: BTreeMap<String, String>,
    pub outputs: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub digest: Digest,
    pub package: Package,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub request_id: Id,
    pub skill: String,
    pub version: String,
    pub input: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub request: Start,
    pub package_digest: Digest,
    pub operation: Operation,
    pub submitted_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub worker: Option<Id>,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<process::Parent>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub run: Id,
    pub worker: Id,
    pub outcome: String,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}
fn invalid(v: impl ToString) -> StoreError {
    StoreError::Invalid(v.to_string())
}
fn key(v: &str) -> Result<Name> {
    Name::new(v).map_err(invalid)
}
fn uid() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn document(schema: &str, value: &impl Serialize) -> Result<Document> {
    Ok(Document {
        schema: key(schema)?,
        value: serde_json::to_value(value).map_err(invalid)?,
    })
}
fn decode<T: serde::de::DeserializeOwned>(row: &Record, schema: &str) -> Result<T> {
    if row.document.schema.as_str() != schema {
        return Err(StoreError::Integrity("skill record schema".into()));
    }
    serde_json::from_value(row.document.value.clone()).map_err(invalid)
}
fn symbol(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}
fn version(s: &str) -> bool {
    s.len() <= 32
        && s.split('.').count() == 3
        && s.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
fn skill_key(name: &str, ver: &str) -> Result<Name> {
    if !symbol(name) || !version(ver) {
        return Err(invalid("name or version invalid"));
    }
    key(&format!("local-sim/skill/{name}/{ver}"))
}
fn run_key(id: &Id) -> Result<Name> {
    key(&format!("local-sim/run/{id}"))
}
fn shape(spec: &BTreeMap<String, String>, value: &Value) -> Result<()> {
    let obj = value
        .as_object()
        .ok_or_else(|| invalid("input/output must be an object"))?;
    if obj.len() != spec.len() {
        return Err(invalid("input/output fields differ from declared schema"));
    }
    for (field, kind) in spec {
        let v = obj
            .get(field)
            .ok_or_else(|| invalid(format!("missing field: {field}")))?;
        let valid = match kind.as_str() {
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "object" => v.is_object(),
            "array" => v.is_array(),
            _ => false,
        };
        if !valid {
            return Err(invalid(format!("field {field} requires {kind}")));
        }
    }
    if canonical::bytes(value).map_err(invalid)?.len() > 65_536 {
        return Err(invalid("input/output exceeds 64 KiB"));
    }
    Ok(())
}
fn schema(spec: &BTreeMap<String, String>) -> Result<()> {
    if spec.len() > 32
        || spec.iter().any(|(n, t)| {
            !symbol(n)
                || !matches!(
                    t.as_str(),
                    "string" | "integer" | "number" | "boolean" | "object" | "array"
                )
        })
    {
        return Err(invalid("invalid field schema"));
    }
    Ok(())
}
fn admit(
    tx: &mut dyn Transaction,
    request: Start,
    now: u64,
    parent: Option<process::Parent>,
) -> Result<Run> {
    if let Some(row) = tx.get(&run_key(&request.request_id)?)? {
        let prior: Run = decode(&row, RUN)?;
        if prior.request != request || prior.parent != parent {
            return Err(StoreError::KeyConflict);
        }
        return Ok(prior);
    }
    process::authorize_child(tx, &request.request_id, parent.as_ref())?;
    let row = tx
        .get(&skill_key(&request.skill, &request.version)?)?
        .ok_or_else(|| invalid("skill version not registered"))?;
    let skill: Skill = decode(&row, SKILL)?;
    shape(&skill.package.inputs, &request.input)?;
    if parent.is_none() && process::allocated_runs(tx)? >= LIMIT {
        return Err(invalid(
            "installation run limit reached; preserve/export this installation",
        ));
    }
    let digest =
        canonical::digest("RX-LOCAL-SIM-RUN-v1", &(&request, skill.digest)).map_err(invalid)?;
    let run = Run {
        operation: Operation::admitted(request.request_id.clone(), digest),
        request,
        package_digest: skill.digest,
        submitted_ms: now,
        started_ms: None,
        finished_ms: None,
        worker: None,
        output: None,
        error: None,
        duration_ms: None,
        parent,
    };
    save_run(tx, &run, None, "ADMITTED")?;
    Ok(run)
}
fn save_run(
    tx: &mut dyn Transaction,
    run: &Run,
    expected: Option<Counter>,
    event: &str,
) -> Result<()> {
    tx.put(
        &run_key(&run.request.request_id)?,
        expected,
        &document(RUN, run)?,
    )?;
    tx.append(
        &uid(),
        &document(
            "rx.local-sim.event.v1",
            &json!({"run":run.request.request_id,"event":event,"operation":run.operation}),
        )?,
    )?;
    Ok(())
}
pub struct Engine<R> {
    repository: R,
}
impl<R: Repository> Engine<R> {
    pub fn open_existing(mut repository: R, now: u64) -> Result<Self> {
        repository.transact(|tx| {
            let row = tx.get(&key("local-sim/installation")?)?.ok_or_else(|| {
                invalid("existing installation metadata missing; initialization refused")
            })?;
            if row.document.schema.as_str() != "rx.local-sim.installation.v1"
                || row
                    .document
                    .value
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(|v| Id::new(v).is_err())
            {
                return Err(invalid("invalid installation identity"));
            }
            Ok(())
        })?;
        Self::open(repository, now)
    }
    pub fn open(mut repository: R, now: u64) -> Result<Self> {
        let pristine = repository.snapshot()?.1.is_empty();
        repository.transact(|tx| {
            let marker=key("local-sim/installation")?;
            if tx.get(&marker)?.is_none() {
                // Never adopt a cell Runtime or another service's database.
                if !pristine { return Err(invalid("not a local-sim installation")); }
                tx.put(&marker,None,&document("rx.local-sim.installation.v1",&json!({"id":uid()}))?)?;
            }
            for row in tx.scan("local-sim/run/")? {
                let mut run:Run=decode(&row,RUN)?;
                if run.started_ms.is_some() && run.operation.phase()!=Phase::Settled {
                    run.operation.lose_continuity().map_err(invalid)?;
                    run.error=Some("server restarted; execution outcome unknown; no automatic replay".into());
                    run.finished_ms=Some(now);
                    let evidence = uid();
                    tx.append(&evidence,&document("rx.local-sim.loss.v1",&json!({"run":run.request.request_id,"reason":"server continuity lost"}))?)?;
                    run.operation.conclude(Conclusion{outcome:Outcome::Unresolved,evidence_ids:vec![evidence]}).map_err(invalid)?;
                    save_run(tx,&run,Some(row.revision),"SERVER_CONTINUITY_LOST")?;
                }
            }
            Ok(())
        })?;
        Ok(Self { repository })
    }
    pub fn register(&mut self, package: Package) -> Result<Skill> {
        let k = skill_key(&package.name, &package.version)?;
        if package.environment != "LOCAL_SIM"
            || !(100..=60_000).contains(&package.timeout_ms)
            || package.code.is_empty()
            || package.code.len() > 262_144
        {
            return Err(invalid(
                "LOCAL_SIM, bounded Python source and 100..60000 ms timeout required",
            ));
        }
        for spec in [&package.inputs, &package.outputs] {
            schema(spec)?;
        }
        let digest = canonical::digest("RX-LOCAL-SIM-SKILL-v1", &package).map_err(invalid)?;
        let skill = Skill { digest, package };
        self.repository.transact(|tx| {
            if let Some(row) = tx.get(&k)? {
                let old: Skill = decode(&row, SKILL)?;
                if old.digest != digest {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(old);
            }
            if tx.scan("local-sim/skill/")?.len() >= 128 {
                return Err(invalid("skill limit reached"));
            }
            tx.put(&k, None, &document(SKILL, &skill)?)?;
            Ok(skill)
        })
    }
    pub fn skills(&mut self) -> Result<Vec<Skill>> {
        self.repository.transact(|tx| {
            tx.scan("local-sim/skill/")?
                .iter()
                .map(|r| decode(r, SKILL))
                .collect()
        })
    }
    pub fn submit(&mut self, request: Start, now: u64) -> Result<Run> {
        self.repository.transact(|tx| admit(tx, request, now, None))
    }
    pub fn runs(&mut self) -> Result<Vec<Run>> {
        self.repository.transact(|tx| {
            tx.scan("local-sim/run/")?
                .iter()
                .map(|r| decode(r, RUN))
                .collect()
        })
    }
    pub fn get(&mut self, id: &Id) -> Result<Run> {
        self.repository.transact(|tx| {
            decode(
                &tx.get(&run_key(id)?)?
                    .ok_or_else(|| invalid("run not found"))?,
                RUN,
            )
        })
    }
    pub fn claim(&mut self, worker: Id, now: u64) -> Result<Option<(Run, Skill)>> {
        self.repository.transact(|tx| {
            process::advance(tx, now)?;
            let rows = tx.scan("local-sim/run/")?;
            let mut queued = Vec::new();
            for row in rows {
                let run: Run = decode(&row, RUN)?;
                if run.started_ms.is_some() && run.operation.phase() != Phase::Settled {
                    return Ok(None);
                }
                if run.started_ms.is_none() {
                    queued.push((row, run));
                }
            }
            queued.sort_by_key(|(_, r)| r.submitted_ms);
            let Some((row, mut run)) = queued.into_iter().next() else {
                return Ok(None);
            };
            let skill: Skill = decode(
                &tx.get(&skill_key(&run.request.skill, &run.request.version)?)?
                    .ok_or_else(|| invalid("skill missing"))?,
                SKILL,
            )?;
            if skill.digest != run.package_digest {
                return Err(StoreError::Integrity("run/package digest differs".into()));
            }
            run.operation.sent().map_err(invalid)?;
            run.worker = Some(worker);
            run.started_ms = Some(now);
            save_run(tx, &run, Some(row.revision), "EXECUTION_ENTERED")?;
            Ok(Some((run, skill)))
        })
    }
    pub fn finish(&mut self, finish: Finish, now: u64) -> Result<Run> {
        self.repository.transact(|tx| {
            let row = tx
                .get(&run_key(&finish.run)?)?
                .ok_or_else(|| invalid("run missing"))?;
            let mut run: Run = decode(&row, RUN)?;
            if run.worker.as_ref() != Some(&finish.worker) {
                return Err(invalid("worker does not own this execution"));
            }
            let receipt = key(&format!("local-sim/result/{}", finish.run))?;
            let doc = document("rx.local-sim.result.v1", &finish)?;
            if let Some(old) = tx.get(&receipt)? {
                if old.document != doc {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(run);
            }
            if run.operation.phase() == Phase::Settled {
                return Err(invalid(
                    "execution already settled; reconciliation required",
                ));
            }
            let outcome = match finish.outcome.as_str() {
                "SUCCEEDED" => Outcome::Succeeded,
                "FAILED" => Outcome::Failed,
                "UNKNOWN" => Outcome::Unresolved,
                _ => return Err(invalid("invalid outcome")),
            };
            if finish.error.as_ref().is_some_and(|e| e.len() > 4096)
                || finish.duration_ms > 86_400_000
            {
                return Err(invalid("result limits"));
            }
            if outcome == Outcome::Succeeded {
                let skill: Skill = decode(
                    &tx.get(&skill_key(&run.request.skill, &run.request.version)?)?
                        .ok_or_else(|| invalid("skill missing"))?,
                    SKILL,
                )?;
                shape(
                    &skill.package.outputs,
                    finish
                        .output
                        .as_ref()
                        .ok_or_else(|| invalid("output missing"))?,
                )?;
                if finish.error.is_some() {
                    return Err(invalid("success cannot contain error"));
                }
            } else if finish.output.is_some() || finish.error.as_ref().is_none_or(|e| e.is_empty())
            {
                return Err(invalid("failure/unknown requires error and no output"));
            }
            let evidence = uid();
            tx.put(&receipt, None, &doc)?;
            tx.append(&evidence, &doc)?;
            run.operation
                .conclude(Conclusion {
                    outcome,
                    evidence_ids: vec![evidence],
                })
                .map_err(invalid)?;
            // A software result does not establish physical/resource handover.
            run.output = finish.output;
            run.error = finish.error;
            run.duration_ms = Some(finish.duration_ms);
            run.finished_ms = Some(now);
            save_run(tx, &run, Some(row.revision), "RESULT_RECORDED")?;
            Ok(run)
        })
    }
    pub fn abandon_worker(&mut self, worker: Id, now: u64) -> Result<usize> {
        self.repository.transact(|tx| {
            let mut count=0;
            for row in tx.scan("local-sim/run/")? {
                let mut run:Run=decode(&row,RUN)?;
                if run.worker.as_ref()==Some(&worker) && run.operation.phase()!=Phase::Settled {
                    run.operation.lose_continuity().map_err(invalid)?;
                    let evidence=uid();
                    tx.append(&evidence,&document("rx.local-sim.loss.v1",&json!({"run":run.request.request_id,"worker":worker,"reason":"worker ownership lost"}))?)?;
                    run.operation.conclude(Conclusion{outcome:Outcome::Unresolved,evidence_ids:vec![evidence]}).map_err(invalid)?;
                    run.finished_ms=Some(now);run.error=Some("worker ownership lost; no automatic replay".into());
                    save_run(tx,&run,Some(row.revision),"WORKER_CONTINUITY_LOST")?;count+=1;
                }
            }
            Ok(count)
        })
    }
}
