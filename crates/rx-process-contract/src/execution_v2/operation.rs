//! Immutable operation-scoped binding sent only through authenticated v2 boundaries.
use super::*;
pub const OPERATION_SCHEMA: &str = "rx.execution-operation.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBinding {
    pub schema: Name,
    pub operation: Id,
    pub mandate: Id,
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub report: ArtifactRef,
    pub selection: Selection,
    pub selection_digest: Digest,
}
impl OperationBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != OPERATION_SCHEMA
            || self.selection.schema.as_str() != "rx.execution-selection.v2"
            || self.selection.ordinal.0 == 0
            || self.selection.slot_ordinal.0 < self.selection.ordinal.0
            || self.selection.slot_ordinal.0 > MAX_SLOTS as u64
            || usize::from(self.selection.slot) >= MAX_SLOTS
            || usize::from(self.selection.candidate) >= MAX_VARIANTS
            || self.selection.object.revision.0 == 0
            || self.selection.authority_generation.0 == 0
            || self.publication.revision != Counter(1)
            || self.publication.id != self.selection.publication
            || self.publication.catalog != self.selection.object.catalog
            || self.report.sha256 != self.selection.report_digest
            || self.selection_digest != self.selection.digest()?
        {
            return Err("execution operation/selection identity differs".into());
        }
        for digest in [
            self.publication.digest,
            self.selection.policy_digest,
            self.selection.configuration_digest,
            self.selection.object.digest,
            self.selection.object_values_digest,
            self.selection.intent_digest,
        ] {
            nonzero(digest)?;
        }
        reference(
            &self.selection.parameter,
            PARAMETER_SCHEMA,
            MAX_PARAMETER_BYTES,
        )?;
        reference(&self.policy, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)?;
        reference(
            &self.report,
            "rx.execution-report.v2",
            MAX_REPORT_BYTES as u64,
        )
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-EXECUTION-OPERATION-v2", self).map_err(|e| e.to_string())
    }
}
impl Selection {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-EXECUTION-SELECTION-v2", self).map_err(|e| e.to_string())
    }
}
