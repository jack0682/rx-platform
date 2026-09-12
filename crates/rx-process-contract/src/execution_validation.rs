//! Validate a typed P snapshot before deriving BT requests. Transport provenance is a separate check.
use crate::{execution::*, frontier, model::*};
use rx_domain::{canonical, types::*};
use std::collections::{BTreeMap, BTreeSet};
fn require(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why.into()) }
}
fn same<T: serde::Serialize>(a: &T, b: &T) -> Result<bool, String> {
    Ok(canonical::bytes(a).map_err(|e| e.to_string())?
        == canonical::bytes(b).map_err(|e| e.to_string())?)
}
pub fn validate(
    snapshot: &ExecutionSnapshot,
    process: &ResolvedProcess,
) -> Result<frontier::Frontier, String> {
    let run = &snapshot.run.run;
    let checkpoint = &snapshot.run.checkpoint;
    require(
        snapshot.schema.as_str() == SNAPSHOT_SCHEMA,
        "unsupported execution snapshot schema",
    )?;
    require(
        snapshot.sequence.0 > 0
            && snapshot.cell_revision.0 > 0
            && snapshot.cell_epoch.0 > 0
            && !snapshot.scope_epochs.is_empty()
            && snapshot.scope_epochs.values().all(|e| e.0 > 0),
        "invalid source position/scope epoch",
    )?;
    require(
        !snapshot.checked_at.clock_id.is_empty()
            && snapshot.checked_at.clock_id == snapshot.valid_until.clock_id
            && snapshot.valid_until.ticks_ns > snapshot.checked_at.ticks_ns
            && snapshot.valid_until.ticks_ns.0 - snapshot.checked_at.ticks_ns.0 <= 100_000_000,
        "invalid snapshot time bound",
    )?;
    require(
        snapshot.run.revision.0 > 0
            && checkpoint.run == run.id
            && checkpoint.revision == snapshot.run.revision
            && checkpoint.executor_schema.as_str() == CHECKPOINT_SCHEMA
            && checkpoint.payload.schema_id.as_str() == CHECKPOINT_SCHEMA,
        "run/checkpoint identity mismatch",
    )?;
    require(
        snapshot.resolved.sha256 == frontier::resolved_digest(process)?
            && snapshot.resolved.schema_id == process.schema
            && snapshot.resolved.size_bytes.0
                == canonical::bytes(process).map_err(|e| e.to_string())?.len() as u64
            && run.recipe_digest == snapshot.resolved.sha256
            && run.envelope_digest == snapshot.envelope.sha256,
        "resolved process/envelope mismatch",
    )?;
    require(
        snapshot.visit.0 > 0
            && snapshot.progress.run == run.id
            && snapshot.progress.resolved_digest == run.recipe_digest
            && snapshot.progress.complete
            && snapshot.process_checkpoint.run == run.id
            && snapshot.process_checkpoint.visit == snapshot.visit,
        "incomplete or foreign process view",
    )?;
    require(
        run.part_ids.iter().collect::<BTreeSet<_>>().len() == run.part_ids.len(),
        "duplicate part identity",
    )?;
    match run.purpose {
        Some(Purpose::Production) => require(
            snapshot.visit.0 <= run.part_ids.len() as u64,
            "visit has no actual part",
        )?,
        Some(Purpose::Setup) => require(snapshot.visit == Counter(1), "setup visit is not one")?,
        _ => return Err("run has no execution purpose".into()),
    }
    require(
        snapshot.request_admission_allowed == snapshot.admission_reason.is_none(),
        "admission flag/reason differ",
    )?;
    if snapshot.request_admission_allowed {
        require(
            run.state == RunState::Executing
                && run.executor_session.as_ref() == Some(&snapshot.caller_session)
                && run.mandate.is_some()
                && run.budget.is_some(),
            "admission flag contradicts run authority",
        )?;
    }
    require(
        same(
            &snapshot.process_checkpoint.branches,
            &snapshot.progress.branches,
        )? && same(&snapshot.process_checkpoint.waits, &snapshot.progress.waits)?,
        "checkpoint/progress decisions differ",
    )?;
    let mut mapped = BTreeMap::new();
    let mut identities = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let mut operations = BTreeSet::new();
    for activation in &checkpoint.activations {
        require(
            activation.visit.0 > 0
                && identities.insert(&activation.id)
                && positions.insert((&activation.node, activation.visit)),
            "duplicate activation identity/position",
        )?;
        let mut slots = BTreeSet::new();
        for slot in &activation.slots {
            require(
                slots.insert(&slot.slot) && operations.insert(&slot.operation),
                "duplicate slot or operation mapping",
            )?;
            if activation.visit == snapshot.visit {
                require(slot.slot.as_str() == "main", "unsupported process slot")?;
                mapped.insert(&activation.node, slot);
            }
        }
    }
    require(
        mapped.len() == snapshot.progress.operations.len(),
        "operation coverage differs from checkpoint",
    )?;
    for (node, progress) in &snapshot.progress.operations {
        let slot = mapped
            .get(node)
            .ok_or("operation is not mapped to a run activation")?;
        require(
            slot.operation == *progress.operation.id()
                && slot.intent_digest == progress.intent_digest
                && progress.intent_digest == progress.operation.intent_digest(),
            "operation identity/intent differs",
        )?;
    }
    for choice in snapshot.process_checkpoint.branches.values() {
        require(
            snapshot
                .process_checkpoint
                .decision_times
                .contains_key(&choice.decision),
            "branch has no P decision time",
        )?;
    }
    for result in snapshot.process_checkpoint.waits.values() {
        let decision = match result {
            frontier::WaitProgress::Satisfied { decision, .. }
            | frontier::WaitProgress::TimedOut { decision } => decision,
        };
        require(
            snapshot
                .process_checkpoint
                .decision_times
                .contains_key(decision),
            "wait has no P decision time",
        )?;
    }
    for window in snapshot.process_checkpoint.wait_windows.values() {
        require(
            window.started_at.clock_id == window.expires_at.clock_id
                && window.started_at.ticks_ns < window.expires_at.ticks_ns,
            "invalid P wait window",
        )?;
    }
    frontier::plan(process, &snapshot.progress)
}
