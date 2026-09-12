//! Discovery of already-started work. This view never allocates a Run or grants admission.
use crate::execution::{Purpose, RunState};
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const SCHEMA: &str = "rx.executor-assignment-state.v1";
pub const MAX_PAYLOAD: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Cardinality {
    None,
    Single,
    Ambiguous,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttemptStatus {
    Pending,
    Arming,
    Started,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAttempt {
    pub id: Id,
    pub status: AttemptStatus,
    pub executor_session: Id,
    pub valid_until: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub run: Id,
    pub revision: Counter,
    pub state: RunState,
    pub purpose: Option<Purpose>,
    pub definition: ArtifactRef,
    pub resolved: ArtifactRef,
    pub executor_session: Option<Id>,
    pub mandate: Option<Id>,
    pub pending_attempt: Option<PendingAttempt>,
    pub configuration_current: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub schema: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub sequence: Counter,
    pub caller_session: Id,
    pub cell: Name,
    pub executor: Name,
    pub definition: ArtifactRef,
    pub cell_revision: Counter,
    pub cell_epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    pub checked_at: TimePoint,
    pub valid_until: TimePoint,
    pub cardinality: Cardinality,
    /// Complete zero/one set, or two distinct witnesses when ambiguous. Never a chosen winner.
    pub candidates: Vec<Candidate>,
}
pub fn validate(view: &View) -> Result<(), String> {
    let expected = match view.cardinality {
        Cardinality::None => 0,
        Cardinality::Single => 1,
        Cardinality::Ambiguous => 2,
    };
    if view.schema.as_str() != SCHEMA
        || view.sequence.0 == 0
        || view.cell_revision.0 == 0
        || view.cell_epoch.0 == 0
        || view.scope_epochs.is_empty()
        || view.scope_epochs.values().any(|v| v.0 == 0)
        || view.definition.size_bytes.0 == 0
        || view.checked_at.clock_id != view.valid_until.clock_id
        || view
            .valid_until
            .ticks_ns
            .0
            .checked_sub(view.checked_at.ticks_ns.0)
            .is_none_or(|n| n == 0 || n > 100_000_000)
        || view.candidates.len() != expected
    {
        return Err("assignment cut/cardinality differs".into());
    }
    let mut seen = BTreeSet::new();
    for candidate in &view.candidates {
        if !seen.insert(&candidate.run)
            || candidate.revision.0 == 0
            || candidate.definition.size_bytes.0 == 0
            || candidate.resolved.size_bytes.0 == 0
            || matches!(candidate.state, RunState::Completed | RunState::Abandoned)
            || (candidate.state == RunState::Prepared && candidate.pending_attempt.is_none())
            || (candidate.state != RunState::Prepared && candidate.pending_attempt.is_some())
            || (candidate.configuration_current && candidate.definition != view.definition)
            || candidate.pending_attempt.as_ref().is_some_and(|a| {
                matches!(a.status, AttemptStatus::Started | AttemptStatus::Rejected)
            })
        {
            return Err("assignment candidate identity/state differs".into());
        }
    }
    Ok(())
}
