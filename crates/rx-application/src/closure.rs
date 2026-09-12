//! Non-operating closure. A clearance cannot authorize motion or a restart.
use crate::{Cell, intervention::CaseRef};
use rx_domain::{condition::Condition, types::*};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopePolicy {
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    /// Verified external containment/isolation and restrictions for remaining out of service.
    pub conditions: Vec<Condition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: Name,
    pub cell: Name,
    pub procedure: ArtifactRef,
    pub external_procedure: ArtifactRef,
    pub dependencies: Vec<ArtifactRef>,
    pub contexts: Vec<ScopePolicy>,
    pub maximum_validity_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Admission {
    pub reference: ArtifactRef,
    pub policy: Policy,
    pub admitted_by: Name,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseRevision {
    pub case: Id,
    pub revision: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareClose {
    pub cell: Name,
    pub expected_cell: Counter,
    pub case_revisions: Vec<CaseRevision>,
    pub evidence_ids: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseWithoutRestart {
    pub cell: Name,
    pub expected_cell: Counter,
    pub case_revisions: Vec<CaseRevision>,
    pub clearance: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundCell {
    pub reference: CaseRef,
    pub revision: Counter,
    pub envelope: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disposition {
    RemainOutOfService,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clearance {
    pub id: Id,
    pub cell: Name,
    pub case_revisions: Vec<CaseRevision>,
    pub contexts: Vec<BoundCell>,
    pub policies: Vec<ArtifactRef>,
    pub evidence_ids: Vec<Id>,
    pub prepared_by: Name,
    pub prepared_at: TimePoint,
    pub valid_until: TimePoint,
    pub disposition: Disposition,
    pub consumed_by: Option<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClosedCell {
    pub revision: Counter,
    pub cell: Cell,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Id,
    pub clearance: Id,
    pub disposition: Disposition,
    pub case_revisions: Vec<CaseRevision>,
    pub cells: Vec<ClosedCell>,
    pub retained_block_ids: Vec<Id>,
    pub closed_by: Name,
    pub closed_at: TimePoint,
}
