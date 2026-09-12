//! Durable plan for transport bootstrap. Bound does not mean qualified or armed.
use rx_domain::{host_snapshot::HostSnapshot, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub host: Name,
    pub platform_session: Id,
    pub snapshot: HostSnapshot,
    pub read_started: TimePoint,
    pub ttl_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: Id,
    pub host: Name,
    pub cell: Name,
    pub producer_session: Id,
    pub host_boot: Id,
    pub evidence_journal: Id,
    pub delivery_journal: Id,
    pub platform_session: Id,
    pub expected_cell: Counter,
    pub definition: Digest,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub source_sessions: BTreeMap<Name, Id>,
    pub block_ids: Vec<Id>,
    pub resources: Vec<Name>,
    pub fence: Counter,
    pub fence_request: Id,
    pub grant_request: Id,
    pub ttl_ms: Counter,
    pub prepared_at: TimePoint,
    pub valid_until: TimePoint,
    pub bound: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Commit {
    pub plan: Id,
    pub fence_receipt: crate::FenceAcknowledgment,
    pub grant: crate::Grant,
    pub grant_sent_at: TimePoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundReceipt {
    pub request_digest: Digest,
    pub registration: crate::HostRegistration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Renewal {
    pub plan: Id,
    pub request: Id,
    pub sequence: Counter,
    pub sent_at: TimePoint,
    pub grant: crate::Grant,
    pub completed: bool,
    pub response_digest: Option<Digest>,
}
