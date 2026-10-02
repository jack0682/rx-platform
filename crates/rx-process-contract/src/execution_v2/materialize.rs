//! Deterministically re-resolve pinned inputs; never accept a caller's report as fact.
use super::*;
use rx_domain::{
    definition::{self, Definition, ValueType},
    workflow::{self, Data, Quantity},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterContract {
    pub unit: Name,
    pub value_type: ValueType,
    pub frame: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeContract {
    pub implementation: String,
    pub version: String,
    pub primitive: Name,
    pub parameters: BTreeMap<Name, ParameterContract>,
}
impl NodeContract {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.implementation.is_empty()
            || self.implementation.len() > 2048
            || self.version.is_empty()
            || self.version.len() > 2048
            || self.parameters.len() > 64
            || self.parameters.values().any(|p| {
                p.frame
                    .as_ref()
                    .is_some_and(|f| f.is_empty() || f.len() > 128)
            })
        {
            return Err("node contract implementation, version, parameter or frame bound".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputClosure {
    pub schema: Name,
    pub workflow: Reference,
    pub label: String,
    pub spec: workflow::Spec,
    pub definitions: Vec<Definition>,
    /// Candidate order is the policy order; slot is zero here and selected later.
    pub requests: Vec<workflow::Request>,
}
impl InputClosure {
    pub fn decode(bytes: &[u8], policy: &Policy) -> Result<Self, String> {
        policy.validate()?;
        verify_artifact(
            bytes,
            &policy.definition_closure,
            MAX_DEFINITION_BYTES as usize,
        )?;
        let value: Self = decode_canonical(bytes, MAX_DEFINITION_BYTES as usize)?;
        value.verify(policy)?;
        Ok(value)
    }
    pub fn artifact(&self) -> Result<ArtifactRef, String> {
        let bytes = canonical::bytes(self).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_DEFINITION_BYTES {
            return Err("input closure exceeds 16 MiB".into());
        }
        Ok(ArtifactRef {
            schema_id: Name::new("rx.execution-input-closure.v2").expect("static schema"),
            sha256: Digest::from_bytes(Sha256::digest(&bytes).into()),
            size_bytes: Counter(bytes.len() as u64),
        })
    }
    fn verify(&self, policy: &Policy) -> Result<BTreeMap<Reference, Definition>, String> {
        if self.schema.as_str() != "rx.execution-input-closure.v2"
            || self.workflow != policy.workflow
            || self.requests.len() != policy.candidates.len()
            || self.definitions.len() > MAX_DEFINITIONS
            || self.artifact()? != policy.definition_closure
        {
            return Err("pinned input closure identity or bounds differ".into());
        }
        self.spec.validate_shape()?;
        let digest = canonical::digest(
            "RX-WORKFLOW-MODEL-v1",
            &(
                &self.workflow.catalog,
                &self.workflow.id,
                self.workflow.revision,
                &self.label,
                &self.spec,
            ),
        )
        .map_err(|e| e.to_string())?;
        if digest != self.workflow.digest {
            return Err("workflow content differs from stored version".into());
        }
        let mut all = BTreeMap::new();
        for definition in &self.definitions {
            definition.verify()?;
            if definition.reference.catalog != self.workflow.catalog
                || all
                    .insert(definition.reference.clone(), definition.clone())
                    .is_some()
            {
                return Err("definition closure catalog or duplicate differs".into());
            }
        }
        for (request, candidate) in self.requests.iter().zip(&policy.candidates) {
            request.validate_shape()?;
            if request.workflow != self.workflow
                || request.slot_index != Counter(0)
                || context_digest(request)? != candidate.context_digest
            {
                return Err("candidate request differs from approved context".into());
            }
            let mut contexts = self.spec.defaults.clone();
            contexts.extend(request.contexts.clone());
            if !contexts
                .values()
                .flatten()
                .any(|r| r == &candidate.object_model)
                || !all
                    .get(&candidate.object_model)
                    .is_some_and(|d| matches!(d.body, definition::Body::ObjectModel { .. }))
            {
                return Err("candidate object model is not the bound object context".into());
            }
        }
        Ok(all)
    }
}

pub fn context_digest(request: &workflow::Request) -> Result<Digest, String> {
    let mut request = request.clone();
    request.slot_index = Counter(0);
    canonical::digest("RX-EXECUTION-CONTEXT-v2", &request).map_err(|e| e.to_string())
}
pub fn compiler_digest() -> Digest {
    canonical::digest(
        "RX-WORKFLOW-MATERIALIZER-v2",
        &(
            include_str!("materialize.rs"),
            include_str!("../execution_v2.rs"),
            include_str!("../../../rx-domain/src/canonical.rs"),
            include_str!("../../../rx-domain/src/types.rs"),
        ),
    )
    .expect("static materializer source")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterValue {
    value: Quantity,
    frame: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    schema: Name,
    inputs: ArtifactRef,
    templates_digest: Digest,
    candidate: u8,
    slot: u16,
    node: Name,
    task: Name,
    primitive: Name,
    values: BTreeMap<Name, ParameterValue>,
    done: workflow::Done,
    on_failure: workflow::KnownFailure,
    on_unknown: workflow::UnknownOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema: Name,
    inputs: ArtifactRef,
    templates_digest: Digest,
    compiler_digest: Digest,
    candidate: u8,
    object_model: Reference,
    slot: u16,
    resolution: workflow::Report,
    actions: BTreeMap<Name, ActionBinding>,
}
pub struct Materialized {
    report: Vec<u8>,
    report_digest: Digest,
    actions: BTreeMap<Name, ActionBinding>,
    parameters: BTreeMap<Name, Vec<u8>>,
}
impl Materialized {
    pub fn report(&self) -> &[u8] {
        &self.report
    }
    pub fn report_digest(&self) -> Digest {
        self.report_digest
    }
    pub fn actions(&self) -> &BTreeMap<Name, ActionBinding> {
        &self.actions
    }
    pub fn parameters(&self) -> &BTreeMap<Name, Vec<u8>> {
        &self.parameters
    }
    pub fn verify_index(
        &self,
        policy: &Policy,
        index: &ValidatedIndex,
        candidate: u8,
        slot: u16,
    ) -> Result<(), String> {
        if index.report(policy.digest()?, candidate, slot)? != self.report_digest {
            return Err("recomputed report differs from prior approval".into());
        }
        Ok(())
    }
}

/// This produces evidence for publication/qualification. It grants no authority.
pub fn materialize(
    policy: &Policy,
    inputs: &InputClosure,
    candidate: u8,
    slot: u16,
) -> Result<Materialized, String> {
    policy.validate()?;
    if policy.compiler_digest != compiler_digest() {
        return Err("materializer implementation differs".into());
    }
    let all = inputs.verify(policy)?;
    if usize::from(candidate) >= policy.candidates.len()
        || usize::from(slot) >= policy.slot_order.len()
    {
        return Err("candidate or slot outside approved domain".into());
    }
    let mut request = inputs.requests[usize::from(candidate)].clone();
    request.slot_index = Counter(u64::from(slot));
    let resolution = workflow::resolve(&inputs.spec, request, &all)?;
    if resolution.resolver_digest != policy.resolver_digest
        || !resolution.valid
        || !resolution.concrete
        || !resolution.violations.is_empty()
        || resolution.status != "RESOLVED_NOT_QUALIFIED"
        || resolution.steps.len() != policy.templates.len()
    {
        return Err(format!(
            "candidate {candidate} slot {slot}: {} or resolver/template mismatch",
            resolution.status
        ));
    }
    let templates_digest = canonical::digest(
        "RX-EXECUTION-TEMPLATES-v2",
        &(&policy.templates, &policy.node_contracts),
    )
    .map_err(|e| e.to_string())?;
    let mut actions = BTreeMap::new();
    let mut parameters = BTreeMap::new();
    for step in &resolution.steps {
        let template = policy.templates.get(&step.node).ok_or("unpublished node")?;
        let contract = &policy.node_contracts[&step.node];
        if step.skills.len() != 1 || actions.contains_key(&step.node) {
            return Err("one Skill per unique node required".into());
        }
        let skill = &step.skills[0];
        let text = |v: &workflow::Resolved| match &v.value.data {
            Data::Text { value } => Some(value.clone()),
            _ => None,
        };
        if text(&skill.implementation).as_deref() != Some(&contract.implementation)
            || text(&skill.version).as_deref() != Some(&contract.version)
            || skill.primitive != contract.primitive
            || skill.parameters.keys().ne(contract.parameters.keys())
        {
            return Err("package Skill contract differs".into());
        }
        for value in step.properties.values() {
            value.value.validate()?;
            if !value.value.concrete() || value.origins.is_empty() {
                return Err("concrete properties with provenance required".into());
            }
        }
        step.done.equals.validate()?;
        if !step.done.equals.concrete() {
            return Err("concrete done condition required".into());
        }
        let timeout = step
            .properties
            .get(&step.timeout_property)
            .ok_or("missing timeout")?;
        let Data::Number { range } = &timeout.value.data else {
            return Err("numeric timeout required".into());
        };
        let millis = range.exact().ok_or("nonconcrete timeout")?
            * match timeout.value.unit.as_str() {
                "s" => 1000.0,
                "ms" => 1.0,
                _ => return Err("timeout unit differs".into()),
            };
        if timeout.frame.is_some() || !millis.is_finite() || millis <= 0.0 || millis.fract() != 0.0
        {
            return Err("timeout exceeds fixed template bound".into());
        }
        let millis: u64 = format!("{millis:.0}")
            .parse()
            .map_err(|_| "timeout overflows integer bound")?;
        if millis > template.intent.execution_timeout_ms.0 {
            return Err("timeout exceeds fixed template bound".into());
        }
        let mut values = BTreeMap::new();
        for (parameter, property) in &skill.parameters {
            let value = step
                .properties
                .get(property)
                .ok_or("missing mapped property")?;
            let expected = &contract.parameters[parameter];
            let kind = match value.value.data {
                Data::Number { .. } => ValueType::Number,
                Data::Vector { .. } => ValueType::Vector,
                Data::Boolean { .. } => ValueType::Boolean,
                Data::Text { .. } => ValueType::Text,
            };
            if kind != expected.value_type
                || value.value.unit != expected.unit
                || value.frame != expected.frame
            {
                return Err(format!(
                    "nodes/{}/properties/{property}: parameter type/unit/frame differs",
                    step.node
                ));
            }
            values.insert(
                parameter.clone(),
                ParameterValue {
                    value: value.value.clone(),
                    frame: value.frame.clone(),
                },
            );
        }
        let payload = Parameters {
            schema: Name::new(PARAMETER_SCHEMA).expect("static schema"),
            inputs: policy.definition_closure.clone(),
            templates_digest,
            candidate,
            slot,
            node: step.node.clone(),
            task: step.task.clone(),
            primitive: skill.primitive.clone(),
            values,
            done: step.done.clone(),
            on_failure: step.on_failure,
            on_unknown: step.on_unknown,
        };
        let bytes = canonical::bytes(&payload).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_PARAMETER_BYTES {
            return Err("node parameter exceeds 64 KiB".into());
        }
        let mut action = template.clone();
        let Body::Program(goal) = &mut action.intent.body else {
            unreachable!("validated Program")
        };
        goal.parameter_set = ArtifactRef {
            schema_id: payload.schema,
            sha256: Digest::from_bytes(Sha256::digest(&bytes).into()),
            size_bytes: Counter(bytes.len() as u64),
        };
        // Only the parameter changes. The package's fixed timeout remains pinned.
        parameters.insert(step.node.clone(), bytes);
        actions.insert(step.node.clone(), action);
    }
    let report = Report {
        schema: Name::new("rx.execution-report.v2").expect("static schema"),
        inputs: policy.definition_closure.clone(),
        templates_digest,
        compiler_digest: policy.compiler_digest,
        candidate,
        object_model: policy.candidates[usize::from(candidate)]
            .object_model
            .clone(),
        slot,
        resolution,
        actions: actions.clone(),
    };
    let bytes = canonical::bytes(&report).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("execution report exceeds 900000 bytes".into());
    }
    Ok(Materialized {
        report_digest: Digest::from_bytes(Sha256::digest(&bytes).into()),
        report: bytes,
        actions,
        parameters,
    })
}
