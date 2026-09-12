//! Run-owned material attempts and a consistent control read. No executor-local completion truth.
use crate::execution::{PartDisposition, Purpose, RunSnapshot, RunState};
use rx_domain::{
    budget::{BudgetUnit, Consumption},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const SCHEMA: &str = "rx.production-state.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: Id,
    pub run: Id,
    pub ordinal: Counter,
    pub revision: Counter,
    pub disposition: PartDisposition,
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
    pub definition: ArtifactRef,
    pub resolved: ArtifactRef,
    pub cell_revision: Counter,
    pub cell_epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    pub checked_at: TimePoint,
    pub valid_until: TimePoint,
    pub run: RunSnapshot,
    pub parts: Vec<Part>,
    pub admission_allowed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletePart {
    pub cell: Name,
    pub run: Id,
    pub part: Id,
    pub expected_run: Counter,
    pub expected_part: Counter,
}
pub fn validate(view: &View) -> Result<(), String> {
    let run = &view.run.run;
    if view.schema.as_str() != SCHEMA
        || view.sequence.0 == 0
        || view.cell_revision.0 == 0
        || view.cell_epoch.0 == 0
        || view.scope_epochs.is_empty()
        || view.scope_epochs.values().any(|v| v.0 == 0)
        || view.checked_at.clock_id != view.valid_until.clock_id
        || view
            .valid_until
            .ticks_ns
            .0
            .checked_sub(view.checked_at.ticks_ns.0)
            .is_none_or(|n| n == 0 || n > 100_000_000)
        || view.run.revision.0 == 0
        || view.run.checkpoint.run != run.id
        || view.run.checkpoint.revision != view.run.revision
        || view.run.checkpoint.executor_schema.as_str() != crate::execution::CHECKPOINT_SCHEMA
        || view.run.checkpoint.payload.schema_id.as_str() != crate::execution::CHECKPOINT_SCHEMA
        || view.resolved.sha256 != run.recipe_digest
        || view.parts.len() != run.part_ids.len()
    {
        return Err("production read identity/cut differs".into());
    }
    let mut ids = BTreeSet::new();
    for (index, part) in view.parts.iter().enumerate() {
        if part.run != run.id
            || part.ordinal.0 != index as u64 + 1
            || part.id != run.part_ids[index]
            || part.revision.0 == 0
            || !ids.insert(&part.id)
        {
            return Err("part coverage/order differs".into());
        }
    }
    if run.purpose == Some(Purpose::Production) {
        let budget = run.budget.as_ref().ok_or("production budget missing")?;
        if budget.unit() != BudgetUnit::PartAttempt
            || budget.consumed().0 != view.parts.len() as u64
            || view
                .parts
                .iter()
                .any(|p| !budget.contains(&Consumption::PartAttempt(p.id.clone())))
        {
            return Err("part budget differs".into());
        }
    } else if !view.parts.is_empty() {
        return Err("parts without production purpose".into());
    }
    if view.admission_allowed
        && (run.state != RunState::Executing
            || run.executor_session.as_ref() != Some(&view.caller_session)
            || run.mandate.is_none())
    {
        return Err("production admission identity differs".into());
    }
    if run.state == RunState::Completed
        && run.purpose == Some(Purpose::Production)
        && (run.budget.as_ref().is_none_or(|b| b.remaining().0 != 0)
            || view
                .parts
                .iter()
                .any(|p| p.disposition != PartDisposition::ConfirmedCompleted))
    {
        return Err("completed production lacks completed parts/exhausted budget".into());
    }
    Ok(())
}
