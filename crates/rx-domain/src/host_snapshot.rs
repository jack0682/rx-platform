//! Read-only Host bootstrap facts. A snapshot never grants execution or establishes completion.
use crate::{DomainError, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservation {
    pub source: Name,
    pub generation: Id,
    pub schema: Name,
    pub unit: Name,
    pub value: TypedValue,
    pub acquired_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
    pub disputed: bool,
    pub evidence_id: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSnapshot {
    pub schema: Name,
    pub host: Name,
    pub host_boot: Id,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub resource_fences: BTreeMap<Name, Counter>,
    pub captured_at: TimePoint,
    pub sources_available: bool,
    pub observations: Vec<SourceObservation>,
    pub block_ids: Vec<Id>,
    pub pending_operations: Vec<Id>,
    pub pending_permits: Vec<Id>,
}
impl HostSnapshot {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema.as_str() != "rx.host-snapshot.v1"
            || !matches!(self.environment.as_str(), "SIMULATION" | "PHYSICAL")
            || self.scopes.is_empty()
            || self.scopes.len() > 128
            || self.resource_fences.len() > 128
            || self.observations.len() > 128
            || self.block_ids.len() > 128
            || self.block_ids.iter().collect::<BTreeSet<_>>().len() != self.block_ids.len()
            || self.pending_operations.len() > 128
            || self.pending_permits.len() > 128
            || (!self.sources_available && !self.observations.is_empty())
            || self
                .observations
                .iter()
                .map(|o| &o.source)
                .collect::<BTreeSet<_>>()
                .len()
                != self.observations.len()
            || self
                .observations
                .iter()
                .map(|o| &o.evidence_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.observations.len()
            || self
                .pending_operations
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.pending_operations.len()
            || self.pending_permits.iter().collect::<BTreeSet<_>>().len()
                != self.pending_permits.len()
        {
            return Err(DomainError::InvalidInput("Host snapshot shape".into()));
        }
        for observation in &self.observations {
            if observation.acquired_at.clock_id != self.captured_at.clock_id
                || observation.acquired_at.ticks_ns > self.captured_at.ticks_ns
            {
                return Err(DomainError::InvalidInput("Host observation clock".into()));
            }
        }
        Ok(())
    }
}
