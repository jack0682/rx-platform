//! Explicit v2 read cut: template identity and Part-specific progress remain distinct.
use super::*;
use crate::{execution, frontier};
pub const SNAPSHOT_SCHEMA: &str = "rx.execution-snapshot.v2";
pub const CONTEXT_SCHEMA: &str = "rx.execution-context.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: Name,
    /// Reuses the fact fields, not the v1 schema or v1 recipe semantics.
    pub context: execution::ExecutionSnapshot,
    pub configuration: ArtifactRef,
    pub plan: Plan,
    pub part: executor::PartBinding,
}
impl Snapshot {
    pub fn validate(&self) -> Result<frontier::Frontier, String> {
        let c = &self.context;
        let run = &c.run.run;
        let cp = &c.run.checkpoint;
        let process = self.plan.instantiate(&self.part)?;
        let projected = frontier::resolved_digest(&process)?;
        if self.schema.as_str() != SNAPSHOT_SCHEMA
            || c.schema.as_str() != CONTEXT_SCHEMA
            || self.part.configuration != self.configuration
            || c.resolved != self.plan.reference()?
            || run.recipe_digest != c.resolved.sha256
            || run.envelope_digest != c.envelope.sha256
            || self.part.run != run.id
            || self.part.ordinal != c.visit
            || c.visit.0 == 0
            || run.part_ids.get((c.visit.0 - 1) as usize) != Some(&self.part.part)
            || run.purpose != Some(execution::Purpose::Production)
            || c.sequence.0 == 0
            || c.cell_revision.0 == 0
            || c.cell_epoch.0 == 0
            || c.scope_epochs.is_empty()
            || c.scope_epochs.values().any(|e| e.0 == 0)
            || c.checked_at.clock_id.is_empty()
            || c.checked_at.clock_id != c.valid_until.clock_id
            || c.valid_until.ticks_ns <= c.checked_at.ticks_ns
            || c.valid_until.ticks_ns.0 - c.checked_at.ticks_ns.0 > 100_000_000
            || c.run.revision.0 == 0
            || cp.run != run.id
            || cp.revision != c.run.revision
            || cp.executor_schema.as_str() != execution::CHECKPOINT_SCHEMA
            || cp.payload.schema_id.as_str() != execution::CHECKPOINT_SCHEMA
            || c.progress.run != run.id
            || c.progress.resolved_digest != projected
            || !c.progress.complete
            || c.process_checkpoint.run != run.id
            || c.process_checkpoint.visit != c.visit
            || !c.progress.branches.is_empty()
            || !c.progress.waits.is_empty()
            || !c.progress.cleared_interventions.is_empty()
            || !c.process_checkpoint.branches.is_empty()
            || !c.process_checkpoint.waits.is_empty()
            || !c.process_checkpoint.wait_windows.is_empty()
            || !c.process_checkpoint.decision_times.is_empty()
            || c.request_admission_allowed != c.admission_reason.is_none()
            || run.part_ids.iter().collect::<BTreeSet<_>>().len() != run.part_ids.len()
        {
            return Err("v2 snapshot identity, time or progress differs".into());
        }
        if c.request_admission_allowed
            && (run.state != execution::RunState::Executing
                || run.executor_session.as_ref() != Some(&c.caller_session)
                || run.mandate.is_none()
                || run.budget.is_none())
        {
            return Err("v2 admission contradicts Run authority".into());
        }
        let mut activations = BTreeSet::new();
        let mut positions = BTreeSet::new();
        let mut operations = BTreeSet::new();
        let mut mapped = BTreeMap::new();
        for a in &cp.activations {
            if a.visit.0 == 0 || !activations.insert(&a.id) || !positions.insert((&a.node, a.visit))
            {
                return Err("duplicate activation identity/position".into());
            }
            let mut slots = BTreeSet::new();
            for s in &a.slots {
                if s.slot.as_str() != "main"
                    || !slots.insert(&s.slot)
                    || !operations.insert(&s.operation)
                {
                    return Err("duplicate or unsupported slot mapping".into());
                }
                if a.visit == c.visit {
                    mapped.insert(&a.node, s);
                }
            }
        }
        if mapped.len() != c.progress.operations.len() {
            return Err("v2 operation coverage differs".into());
        }
        for (node, p) in &c.progress.operations {
            let s = mapped.get(node).ok_or("unmapped v2 operation")?;
            if s.operation != *p.operation.id()
                || s.intent_digest != p.intent_digest
                || p.intent_digest != p.operation.intent_digest()
            {
                return Err("v2 operation identity differs".into());
            }
        }
        frontier::plan(&process, &c.progress)
    }
}
