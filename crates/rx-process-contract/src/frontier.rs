//! Pure planning from a validated P view. Suggestions never grant native or workflow authority.
use crate::model::*;
use rx_domain::{
    canonical,
    operation::{Disposition, Integrity, Knowledge, Operation, Outcome},
    types::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationProgress {
    pub operation: Operation,
    pub intent_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchChoice {
    pub decision: Id,
    pub chosen: bool,
    pub evidence_ids: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum WaitProgress {
    Satisfied { decision: Id, evidence_ids: Vec<Id> },
    TimedOut { decision: Id },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressView {
    pub run: Id,
    pub resolved_digest: Digest,
    pub complete: bool,
    pub operations: BTreeMap<Name, OperationProgress>,
    pub branches: BTreeMap<Name, BranchChoice>,
    pub waits: BTreeMap<Name, WaitProgress>,
    pub cleared_interventions: BTreeMap<Name, Id>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Ready,
    Running,
    Blocked,
    Failed,
    Completed,
}
#[derive(Clone, Debug, Serialize)]
pub struct Frontier {
    pub state: State,
    pub operations: Vec<Name>,
    pub decisions: Vec<Name>,
    pub waits: Vec<Name>,
    pub handovers: Vec<Name>,
    pub interventions: Vec<Name>,
    pub pending_operations: Vec<Id>,
    pub reasons: Vec<String>,
}
impl Frontier {
    fn state(state: State) -> Self {
        Self {
            state,
            operations: vec![],
            decisions: vec![],
            waits: vec![],
            handovers: vec![],
            interventions: vec![],
            pending_operations: vec![],
            reasons: vec![],
        }
    }
    fn merge(&mut self, mut other: Self) {
        self.operations.append(&mut other.operations);
        self.decisions.append(&mut other.decisions);
        self.waits.append(&mut other.waits);
        self.handovers.append(&mut other.handovers);
        self.interventions.append(&mut other.interventions);
        self.pending_operations
            .append(&mut other.pending_operations);
        self.reasons.append(&mut other.reasons);
    }
    fn suppress_new(&mut self) {
        self.operations.clear();
        self.decisions.clear();
        self.waits.clear();
    }
}
pub fn resolved_digest(process: &ResolvedProcess) -> Result<Digest, String> {
    let mut nodes = BTreeMap::new();
    collect(&process.root, &mut nodes, 0)?;
    Ok(Digest::from_bytes(
        Sha256::digest(canonical::bytes(process).map_err(|e| e.to_string())?).into(),
    ))
}
pub fn plan(process: &ResolvedProcess, view: &ProgressView) -> Result<Frontier, String> {
    if !view.complete {
        return Err("partial progress cannot drive a process".into());
    }
    if view.resolved_digest != resolved_digest(process)? {
        return Err("progress belongs to a different resolved process".into());
    }
    let mut nodes = BTreeMap::new();
    collect(&process.root, &mut nodes, 0)?;
    let mut ids = BTreeSet::new();
    for (node, progress) in &view.operations {
        let Some(CompiledNode {
            body: CompiledBody::Operation { binding },
            ..
        }) = nodes.get(node).copied()
        else {
            return Err("operation progress targets a non-operation node".into());
        };
        let binding = process
            .bindings
            .get(binding)
            .ok_or("compiled action binding missing")?;
        if progress.intent_digest != binding.intent.digest().map_err(|e| e.to_string())?
            || !ids.insert(progress.operation.id())
        {
            return Err("operation intent/activation correlation differs".into());
        }
    }
    for (node, choice) in &view.branches {
        if !matches!(
            nodes.get(node).map(|n| &n.body),
            Some(CompiledBody::Branch { .. })
        ) || choice.evidence_ids.is_empty()
        {
            return Err("branch choice requires a matching committed decision and evidence".into());
        }
    }
    for (node, wait) in &view.waits {
        if !matches!(
            nodes.get(node).map(|n| &n.body),
            Some(CompiledBody::Wait { .. })
        ) || matches!(wait,WaitProgress::Satisfied {evidence_ids,..} if evidence_ids.is_empty())
        {
            return Err("wait progress has no matching evidenced decision".into());
        }
    }
    for node in view.cleared_interventions.keys() {
        if !matches!(
            nodes.get(node).map(|n| &n.body),
            Some(CompiledBody::Intervention { .. })
        ) {
            return Err("clearance targets a non-intervention node".into());
        }
    }
    evaluate(&process.root, view)
}
fn collect<'a>(
    node: &'a CompiledNode,
    all: &mut BTreeMap<Name, &'a CompiledNode>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 || all.len() >= 4096 || all.insert(node.id.clone(), node).is_some() {
        return Err("compiled node depth/count/identity differs".into());
    }
    match &node.body {
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            if children.is_empty() {
                return Err("empty compiled control node".into());
            }
            for child in children {
                collect(child, all, depth + 1)?;
            }
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => {
            collect(when_true, all, depth + 1)?;
            collect(when_false, all, depth + 1)?;
        }
        _ => {}
    }
    Ok(())
}
fn evaluate(node: &CompiledNode, view: &ProgressView) -> Result<Frontier, String> {
    match &node.body {
        CompiledBody::Operation { .. } => {
            let Some(progress) = view.operations.get(&node.id) else {
                let mut result = Frontier::state(State::Ready);
                result.operations.push(node.id.clone());
                return Ok(result);
            };
            let operation = &progress.operation;
            if operation.integrity() == Integrity::Disputed
                || operation.outcome() == Outcome::Unresolved
                || operation.knowledge() == Knowledge::Unknown
            {
                let mut result = Frontier::state(State::Blocked);
                result
                    .reasons
                    .push(format!("{} requires reconciliation", node.id));
                result.pending_operations.push(operation.id().clone());
                return Ok(result);
            }
            match operation.outcome() {
                Outcome::Succeeded if operation.disposition() == Disposition::Released => {
                    Ok(Frontier::state(State::Completed))
                }
                Outcome::Succeeded => {
                    let mut result = Frontier::state(State::Running);
                    result.handovers.push(node.id.clone());
                    Ok(result)
                }
                Outcome::Failed | Outcome::Canceled | Outcome::NotExecuted => {
                    Ok(Frontier::state(State::Failed))
                }
                _ => {
                    let mut result = Frontier::state(State::Running);
                    result.pending_operations.push(operation.id().clone());
                    Ok(result)
                }
            }
        }
        CompiledBody::Sequence { children } => {
            for (index, child) in children.iter().enumerate() {
                let result = evaluate(child, view)?;
                if result.state != State::Completed {
                    if children[index + 1..]
                        .iter()
                        .any(|later| has_progress(later, view))
                    {
                        return Err("progress crosses an incomplete sequence predecessor".into());
                    }
                    return Ok(result);
                }
            }
            Ok(Frontier::state(State::Completed))
        }
        CompiledBody::ParallelAll { children } => {
            let results = children
                .iter()
                .map(|child| evaluate(child, view))
                .collect::<Result<Vec<_>, _>>()?;
            let blocked = results.iter().any(|r| r.state == State::Blocked);
            let failed = results.iter().any(|r| r.state == State::Failed);
            let all_done = results.iter().all(|r| r.state == State::Completed);
            let mut result = Frontier::state(State::Running);
            for child in results {
                result.merge(child);
            }
            result.state = if blocked || (failed && !result.pending_operations.is_empty()) {
                State::Blocked
            } else if failed {
                State::Failed
            } else if all_done {
                State::Completed
            } else if !result.operations.is_empty() {
                State::Ready
            } else {
                State::Running
            };
            if blocked || failed {
                result.suppress_new();
            }
            Ok(result)
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => {
            if let Some(choice) = view.branches.get(&node.id) {
                if has_progress(if choice.chosen { when_false } else { when_true }, view) {
                    return Err("progress exists on the unselected branch".into());
                }
                evaluate(if choice.chosen { when_true } else { when_false }, view)
            } else {
                if has_progress(when_true, view) || has_progress(when_false, view) {
                    return Err("branch activity lacks a committed choice".into());
                }
                let mut result = Frontier::state(State::Running);
                result.decisions.push(node.id.clone());
                Ok(result)
            }
        }
        CompiledBody::Wait { .. } => match view.waits.get(&node.id) {
            Some(WaitProgress::Satisfied { .. }) => Ok(Frontier::state(State::Completed)),
            Some(WaitProgress::TimedOut { .. }) => Ok(Frontier::state(State::Failed)),
            None => {
                let mut result = Frontier::state(State::Running);
                result.waits.push(node.id.clone());
                Ok(result)
            }
        },
        CompiledBody::Intervention { .. } => {
            if view.cleared_interventions.contains_key(&node.id) {
                Ok(Frontier::state(State::Completed))
            } else {
                let mut result = Frontier::state(State::Blocked);
                result.interventions.push(node.id.clone());
                Ok(result)
            }
        }
    }
}
fn has_progress(node: &CompiledNode, view: &ProgressView) -> bool {
    if view.operations.contains_key(&node.id)
        || view.branches.contains_key(&node.id)
        || view.waits.contains_key(&node.id)
        || view.cleared_interventions.contains_key(&node.id)
    {
        return true;
    }
    match &node.body {
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            children.iter().any(|child| has_progress(child, view))
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => has_progress(when_true, view) || has_progress(when_false, view),
        _ => false,
    }
}
