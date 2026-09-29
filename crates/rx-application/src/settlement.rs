//! Current non-actuating authority for settling already-confirmed work.
//! This is not a permit, an Arm, a qualification, or a claim that UNKNOWN means failure.
use crate::host_recovery::StoredActor;
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approve {
    pub operation: Id,
    pub expected_operation: Counter,
    pub expected_cell: Counter,
    pub justification: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub id: Id,
    pub operation: Id,
    pub operation_revision: Counter,
    pub intent_digest: Digest,
    pub runtime_boot: Id,
    pub cell: Name,
    pub cell_epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub configuration: Digest,
    pub host: Name,
    pub host_boot: Id,
    pub host_session: Id,
    pub host_journal: Id,
    pub resource_fences: BTreeMap<Name, Counter>,
    pub approved_by: StoredActor,
    pub approved_at: TimePoint,
    pub valid_until: TimePoint,
    pub justification: String,
    pub applied_at: Option<TimePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejoin: Option<crate::host_rejoin::SettlementReference>,
}
