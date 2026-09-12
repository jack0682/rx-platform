//! Intervention records are not physical access certificates or restart permissions.
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaseType {
    DiagnosticOnly,
    PlannedAccess,
    FaultRecovery,
    Maintenance,
    ChangeReview,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaseState {
    Open,
    ContainmentPending,
    ProcedureActive,
    Revalidating,
    ReadyForRestart,
    Closed,
    Escalated,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseRef {
    pub cell: Name,
    pub definition: Digest,
    pub cell_epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: Id,
    pub cell: Name,
    pub kind: CaseType,
    pub state: CaseState,
    pub procedure: ArtifactRef,
    pub lead: Name,
    pub participants: Vec<Name>,
    pub operation_ids: Vec<Id>,
    pub material_ids: Vec<Id>,
    pub record_ids: Vec<Id>,
    pub block_ids: Vec<Id>,
    pub requested_scopes: Vec<Name>,
    pub effective_cells: Vec<Name>,
    pub scope_uncertain: bool,
    pub opened_by: Name,
    pub opened_at: TimePoint,
    pub reference: CaseRef,
    pub related_runs: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSnapshot {
    pub revision: Counter,
    pub case: Case,
    pub current: CaseRef,
    #[serde(default = "zero")]
    pub acknowledgment_count: Counter,
    #[serde(default = "zero")]
    pub procedure_record_count: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCase {
    pub cell: Name,
    #[serde(default)]
    pub expected_cell: Option<Counter>,
    pub kind: CaseType,
    pub scopes: Vec<Name>,
    pub procedure: ArtifactRef,
    pub lead: Name,
    pub operation_ids: Vec<Id>,
    pub material_ids: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeCase {
    pub cell: Name,
    pub case: Id,
    pub expected_case: Counter,
    pub occurred_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledgment {
    pub id: Id,
    pub case: Id,
    pub case_revision: Counter,
    pub actor: Name,
    pub occurred_at: String,
    pub recorded_at: TimePoint,
    pub scopes: Vec<Name>,
    pub assertions: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseDetail {
    #[serde(default)]
    pub procedure_progress: crate::procedure::Progress,
    pub snapshot: CaseSnapshot,
    pub acknowledgments: Vec<Acknowledgment>,
    #[serde(default)]
    pub procedure_records: Vec<crate::procedure::StoredRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseList {
    pub cell: Name,
    pub cases: Vec<CaseSnapshot>,
    pub truncated: bool,
}

fn zero() -> Counter {
    Counter(0)
}
