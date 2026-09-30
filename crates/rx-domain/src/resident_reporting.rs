//! Attributed Supervisor registry observations, never execution or device permission.
use crate::{
    component::{Binding, CatalogReference},
    types::*,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: Id,
    pub principal: Name,
    pub peer_boot: Id,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub authentication_binding: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub id: Id,
    pub component: Id,
    pub component_revision: Counter,
    pub source_registration: Id,
    pub source_revision: Counter,
    pub catalog: CatalogReference,
    pub reporter_session: Id,
    pub issued_by: Name,
    pub issued_at: TimePoint,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionState {
    Assigned,
    Running,
    Exited,
    NotStarted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub scope: Id,
    pub source: Binding,
    pub sequence: Counter,
    pub state: ExecutionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Basis {
    ReportedRegistrySnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Ownership {
    NotEstablishedByReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkUse {
    NotEvaluated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: Id,
    pub component: Id,
    pub component_revision: Counter,
    pub reporter: Peer,
    pub report: Report,
    /// P's receipt time; not the time at which the process state was observed.
    pub accepted_at: TimePoint,
    pub basis: Basis,
    pub execution_ownership: Ownership,
    pub work_use_permission: WorkUse,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub receipt: Receipt,
    pub reporter_session_current: bool,
    pub registration_revision_current: bool,
}
