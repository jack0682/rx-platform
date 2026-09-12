use super::*;
use rx_process_contract::{
    CompiledBody,
    frontier::{self, BranchChoice, WaitProgress},
};
pub(super) enum Transition {
    Applied,
    Waiting,
    Changed(ProcessCheckpoint),
}
/// Both preparation and commit derive a full successor from current P state.
/// During commit only IDs and P-issued preparation time are reused, never a caller's verdict.
pub(super) fn transition(
    tx: &mut dyn Transaction,
    run: &Run,
    cell: &Cell,
    target: &CheckpointTarget,
    now: &TimePoint,
    prepared_at: &TimePoint,
    seed: Option<&ProcessCheckpoint>,
) -> Result<Transition> {
    let process = cell
        .configuration
        .process
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    let (mut cp, view) = super::process::process_view(tx, run, cell, target.visit)?;
    let next = frontier::plan(process, &view).map_err(StoreError::Integrity)?;
    let node = rx_process_contract::validation::nodes(process)
        .into_iter()
        .find(|n| n.id == target.node)
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let seed_decision = |wait: &WaitProgress| match wait {
        WaitProgress::Satisfied { decision, .. } | WaitProgress::TimedOut { decision } => {
            decision.clone()
        }
    };
    match (&target.action, &node.body) {
        (CheckpointAction::ChooseBranch, CompiledBody::Branch { condition, .. }) => {
            if cp.branches.contains_key(&target.node) {
                return Ok(Transition::Applied);
            }
            if !next.decisions.contains(&target.node) {
                return reject(Reject::ConditionUnknown);
            }
            let result = evaluate_raw(tx, cell, &[process.conditions[condition].clone()], now)?;
            if result.verdict == Verdict::Unknown || result.evidence_ids.is_empty() {
                return Ok(Transition::Waiting);
            }
            let choice = BranchChoice {
                decision: seed
                    .and_then(|s| s.branches.get(&target.node))
                    .map(|s| s.decision.clone())
                    .unwrap_or_else(id),
                chosen: result.verdict == Verdict::Pass,
                evidence_ids: result.evidence_ids,
            };
            cp.decision_times
                .insert(choice.decision.clone(), prepared_at.clone());
            cp.branches.insert(target.node.clone(), choice);
        }
        (CheckpointAction::StartWait, CompiledBody::Wait { timeout_ns, .. }) => {
            if cp.wait_windows.contains_key(&target.node) {
                return Ok(Transition::Applied);
            }
            if !next.waits.contains(&target.node) {
                return reject(Reject::ConditionUnknown);
            }
            let expires = prepared_at
                .ticks_ns
                .0
                .checked_add(timeout_ns.0)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let window = WaitWindow {
                id: seed
                    .and_then(|s| s.wait_windows.get(&target.node))
                    .map(|s| s.id.clone())
                    .unwrap_or_else(id),
                started_at: prepared_at.clone(),
                expires_at: TimePoint {
                    clock_id: prepared_at.clock_id.clone(),
                    ticks_ns: Counter(expires),
                },
            };
            cp.wait_windows.insert(target.node.clone(), window);
        }
        (CheckpointAction::CheckWait, CompiledBody::Wait { condition, .. }) => {
            if cp.waits.contains_key(&target.node) {
                return Ok(Transition::Applied);
            }
            if !next.waits.contains(&target.node) {
                return reject(Reject::ConditionUnknown);
            }
            let window = cp
                .wait_windows
                .get(&target.node)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if window.expires_at.clock_id != now.clock_id {
                return reject(Reject::ContinuityUnproven);
            }
            let decision = seed
                .and_then(|s| s.waits.get(&target.node))
                .map(seed_decision)
                .unwrap_or_else(id);
            let result = if now.ticks_ns >= window.expires_at.ticks_ns {
                WaitProgress::TimedOut { decision }
            } else {
                let result = evaluate_raw(tx, cell, &[process.conditions[condition].clone()], now)?;
                if result.verdict != Verdict::Pass || result.evidence_ids.is_empty() {
                    return Ok(Transition::Waiting);
                }
                WaitProgress::Satisfied {
                    decision,
                    evidence_ids: result.evidence_ids,
                }
            };
            cp.decision_times
                .insert(seed_decision(&result), prepared_at.clone());
            cp.waits.insert(target.node.clone(), result);
        }
        _ => return reject(Reject::InvalidInput),
    }
    cp.revision = cp.revision.increment().map_err(domain_error)?;
    Ok(Transition::Changed(cp))
}
