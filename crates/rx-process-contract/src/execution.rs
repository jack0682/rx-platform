//! Shared checkpoint and execution-read DTOs. These data objects never grant P authority.
use rx_domain::{budget::RunBudget, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const CHECKPOINT_SCHEMA: &str = "rx.executor-state.v1";
pub const SNAPSHOT_SCHEMA: &str = "rx.execution-snapshot.v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Purpose {
    Production,
    Setup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunState {
    Prepared,
    Executing,
    Paused,
    RecoveryRequired,
    Completed,
    Abandoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PartDisposition {
    InProgress,
    ConfirmedCompleted,
    Rejected,
    Unresolved,
    NotProcessed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: Id,
    pub cell: Name,
    pub recipe_digest: Digest,
    pub envelope_digest: Digest,
    pub purpose: Option<Purpose>,
    pub state: RunState,
    pub budget: Option<RunBudget>,
    pub executor_session: Option<Id>,
    pub mandate: Option<Id>,
    pub part_ids: Vec<Id>,
    pub pending_attempt: Option<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCheckpoint {
    #[serde(default)]
    pub decision_times: BTreeMap<Id, TimePoint>,
    pub run: Id,
    pub visit: Counter,
    pub revision: Counter,
    pub branches: BTreeMap<Name, crate::frontier::BranchChoice>,
    pub waits: BTreeMap<Name, crate::frontier::WaitProgress>,
    pub wait_windows: BTreeMap<Name, WaitWindow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitWindow {
    pub id: Id,
    pub started_at: TimePoint,
    pub expires_at: TimePoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotSnapshot {
    pub slot: Name,
    pub operation: Id,
    pub intent_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationSnapshot {
    pub id: Id,
    pub node: Name,
    pub visit: Counter,
    pub slots: Vec<SlotSnapshot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorState {
    pub schema: Name,
    pub run: Run,
    pub revision: Counter,
    pub activations: Vec<ActivationSnapshot>,
    pub process_checkpoints: Vec<ProcessCheckpoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointView {
    pub run: Id,
    pub revision: Counter,
    pub executor_schema: Name,
    pub payload: ArtifactRef,
    pub activations: Vec<ActivationSnapshot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSnapshot {
    pub revision: Counter,
    pub run: Run,
    pub checkpoint: CheckpointView,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSnapshot {
    pub schema: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub sequence: Counter,
    pub caller_session: Id,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub cell_revision: Counter,
    pub cell_epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    pub checked_at: TimePoint,
    pub valid_until: TimePoint,
    pub run: RunSnapshot,
    pub visit: Counter,
    pub resolved: ArtifactRef,
    pub process_checkpoint: ProcessCheckpoint,
    pub progress: crate::frontier::ProgressView,
    pub request_admission_allowed: bool,
    pub admission_reason: Option<rx_domain::fault::Rejection>,
}
