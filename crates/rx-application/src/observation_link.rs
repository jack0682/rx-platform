//! Read-only registration, separate from Host operation grants.
//! Register is an internal pinned-transport input, never a caller-supplied HTTP snapshot.
use crate::host_binding_baseline::BootstrapProvenance;
use rx_domain::{host_snapshot::HostSnapshot, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Register {
    pub host: Name,
    pub platform_session: Id,
    pub read_started: TimePoint,
    pub provenance: BootstrapProvenance,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub id: Id,
    pub host: Name,
    pub cell: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub producer_session: Id,
    pub producer_authentication_binding: Digest,
    pub platform_session: Id,
    pub configuration_digest: Digest,
    pub source_sessions: BTreeMap<Name, Id>,
    pub created_at: TimePoint,
    pub provenance: BootstrapProvenance,
}
#[derive(Clone, Debug)]
pub struct Read {
    pub link: Id,
    pub snapshot: HostSnapshot,
    pub read_started: TimePoint,
}
