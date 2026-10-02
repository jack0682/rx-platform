//! P-owned simulation slot custody. Reservations are not operation permits.
use rx_domain::{definition::Reference, types::*};
use rx_process_contract::execution_v2::SlotResource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Initialize {
    pub cell: Name,
    pub resource: Reference,
    pub rule: Reference,
    pub expected_generation: Option<Counter>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    pub run: Id,
    pub ordinal: Counter,
    pub slot_ordinal: Counter,
    pub part: Option<Id>,
    pub consumed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pool {
    pub cell: Name,
    pub layout: SlotResource,
    pub generation: Counter,
    pub holds: BTreeMap<u16, Hold>,
    pub actor: Name,
    pub recorded_at: TimePoint,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRun {
    pub cell: Name,
    pub publication: Reference,
    pub expected_cell: Counter,
    pub count: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolBinding {
    pub resource: Reference,
    pub rule: Reference,
    pub generation: Counter,
    pub layout_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub ordinal: Counter,
    pub slot_ordinal: Counter,
    pub index: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBinding {
    pub run: Id,
    pub cell: Name,
    pub configuration: ArtifactRef,
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub pools: Vec<PoolBinding>,
    pub slots: Vec<Slot>,
    pub actor: Name,
    pub request: Id,
    pub created_at: TimePoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindObject {
    pub run: Id,
    pub ordinal: Counter,
    pub object: Reference,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBinding {
    pub run: Id,
    pub cell: Name,
    pub ordinal: Counter,
    pub slot_ordinal: Counter,
    pub slot: u16,
    pub object: Reference,
    pub model: Reference,
    pub context: Name,
    pub candidate: u8,
    pub values_digest: Digest,
    pub sources: BTreeMap<Name, Reference>,
    pub actor: Name,
    pub request: Id,
    pub created_at: TimePoint,
}
