//! A prepared state is a proposal, never permission or an applied process decision.
use crate::execution::CheckpointView;
use rx_domain::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckpointAction {
    ChooseBranch,
    StartWait,
    CheckWait,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointTarget {
    pub run: Id,
    pub node: Name,
    pub visit: Counter,
    pub action: CheckpointAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedCheckpoint {
    pub expected_revision: Counter,
    pub checkpoint: CheckpointView,
    pub prepared_at: TimePoint,
    pub valid_until: TimePoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum CheckpointPreparation {
    Ready { proposal: Box<PreparedCheckpoint> },
    Waiting { run_revision: Counter },
    AlreadyApplied { run_revision: Counter },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitCheckpoint {
    pub run: Id,
    pub expected_revision: Counter,
    pub new_checkpoint: CheckpointView,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum CheckpointDecision {
    Branch(crate::frontier::BranchChoice),
    WaitWindow(crate::execution::WaitWindow),
    WaitResult(crate::frontier::WaitProgress),
}
impl CheckpointDecision {
    pub fn action(&self) -> CheckpointAction {
        match self {
            Self::Branch(_) => CheckpointAction::ChooseBranch,
            Self::WaitWindow(_) => CheckpointAction::StartWait,
            Self::WaitResult(_) => CheckpointAction::CheckWait,
        }
    }
    pub fn at(
        checkpoint: &crate::execution::ProcessCheckpoint,
        target: &CheckpointTarget,
    ) -> Option<Self> {
        if checkpoint.run != target.run || checkpoint.visit != target.visit {
            return None;
        }
        match target.action {
            CheckpointAction::ChooseBranch => checkpoint
                .branches
                .get(&target.node)
                .cloned()
                .map(Self::Branch),
            CheckpointAction::StartWait => checkpoint
                .wait_windows
                .get(&target.node)
                .cloned()
                .map(Self::WaitWindow),
            CheckpointAction::CheckWait => checkpoint
                .waits
                .get(&target.node)
                .cloned()
                .map(Self::WaitResult),
        }
    }
}
