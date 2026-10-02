//! Explicit Executor-v2 negotiation and immutable Part data. DTOs grant no authority.
use super::*;
pub const SESSION_SCHEMA: &str = "rx.execution-session.v2";
pub const PART_SCHEMA: &str = "rx.execution-part.v2";
pub const PART_BINDING_SCHEMA: &str = "rx.execution-part-binding.v2";
pub fn binding_hash() -> Digest {
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../spec/executor-execution/v2/binding.json"
    ))
    .expect("binding manifest");
    canonical::digest("RX-EXECUTOR-EXECUTION-BINDING-v2", &manifest).expect("static binding")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub schema: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub principal: Name,
    pub session: Id,
    pub cell: Name,
    pub definition: Digest,
    pub binding: Digest,
    pub declared_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartBinding {
    pub schema: Name,
    pub run: Id,
    pub part: Id,
    pub ordinal: Counter,
    pub slot_ordinal: Counter,
    pub slot: u16,
    pub object: Reference,
    pub model: Reference,
    pub object_values_digest: Digest,
    pub candidate: u8,
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub configuration: ArtifactRef,
    pub report: ArtifactRef,
    pub parameters: BTreeMap<Name, ArtifactRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub schema: Name,
    pub binding: PartBinding,
    pub state: crate::production::Part,
}

impl Session {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != SESSION_SCHEMA || self.binding != binding_hash() {
            return Err("execution session protocol differs".into());
        }
        nonzero(self.definition)
    }
}
impl PartBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != PART_BINDING_SCHEMA
            || self.ordinal.0 == 0
            || self.slot_ordinal.0 < self.ordinal.0
            || self.slot_ordinal.0 > MAX_SLOTS as u64
            || usize::from(self.slot) >= MAX_SLOTS
            || usize::from(self.candidate) >= MAX_VARIANTS
            || self.object.revision.0 == 0
            || self.model.revision.0 == 0
            || self.publication.revision != Counter(1)
            || self.object.catalog != self.publication.catalog
            || self.model.catalog != self.publication.catalog
            || self.parameters.is_empty()
            || self.parameters.len() > MAX_NODES
        {
            return Err("execution Part binding identity/domain differs".into());
        }
        nonzero(self.object.digest)?;
        nonzero(self.model.digest)?;
        nonzero(self.publication.digest)?;
        nonzero(self.object_values_digest)?;
        reference(&self.policy, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)?;
        reference(&self.configuration, "rx.cell-configuration.v2", 1_048_576)?;
        reference(
            &self.report,
            "rx.execution-report.v2",
            MAX_REPORT_BYTES as u64,
        )?;
        for r in self.parameters.values() {
            reference(r, PARAMETER_SCHEMA, MAX_PARAMETER_BYTES)?;
        }
        Ok(())
    }
}
impl Part {
    pub fn validate(&self) -> Result<(), String> {
        self.binding.validate()?;
        if self.schema.as_str() != PART_SCHEMA
            || self.state.revision.0 == 0
            || self.state.id != self.binding.part
            || self.state.run != self.binding.run
            || self.state.ordinal != self.binding.ordinal
        {
            return Err("execution Part snapshot identity differs".into());
        }
        Ok(())
    }
}

pub const ADMISSION_SCHEMA: &str = "rx.execution-admission.v2";
/// Public receipt projection; excludes P's internal Work/Host bookkeeping fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    pub schema: Name,
    pub binding: OperationBinding,
    pub operation: rx_domain::operation::Operation,
    pub activation: Id,
    pub permit: Id,
    pub host: Name,
}
impl Admission {
    pub fn validate(&self) -> Result<(), String> {
        self.binding.validate()?;
        if self.schema.as_str() != ADMISSION_SCHEMA
            || self.binding.operation != *self.operation.id()
            || self.binding.selection.intent_digest != self.operation.intent_digest()
        {
            return Err("execution admission identity differs".into());
        }
        Ok(())
    }
}
