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

pub enum PartPreparation {
    Recorded(Box<rx_process_contract::execution_v2::executor::Part>),
    Compute(Box<PartTicket>),
}
pub struct PartTicket {
    pub(crate) identity: crate::Identity,
    pub(crate) key: Id,
    pub(crate) command: crate::BeginPartRequest,
    pub(crate) run_revision: Counter,
    pub(crate) object: ObjectBinding,
    pub(crate) binding: RunBinding,
    pub(crate) policy: rx_process_contract::execution_v2::Policy,
    pub(crate) inputs: rx_process_contract::execution_v2::InputClosure,
    pub(crate) index: rx_process_contract::execution_v2::ValidatedIndex,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
}
pub struct PreparedPart {
    pub(crate) ticket: PartTicket,
    pub(crate) materialized: rx_process_contract::execution_v2::Materialized,
}
impl PreparedPart {
    pub fn prepare(ticket: PartTicket) -> Result<Self, String> {
        let materialized = rx_process_contract::execution_v2::materialize(
            &ticket.policy,
            &ticket.inputs,
            ticket.object.candidate,
            ticket.object.slot,
        )?;
        materialized.verify_index(
            &ticket.policy,
            &ticket.index,
            ticket.object.candidate,
            ticket.object.slot,
        )?;
        Ok(Self {
            ticket,
            materialized,
        })
    }
}

/// Node intent and values are selected by P, never supplied by the Executor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitNode {
    pub cell: Name,
    pub run: Id,
    pub part: Id,
    pub node: Name,
    pub mandate: Id,
    pub expected_cell: Counter,
    pub expected_run: Counter,
}
pub enum OperationPreparation {
    Recorded(Box<crate::Work>),
    Compute(Box<OperationTicket>),
}
pub struct OperationTicket {
    pub(crate) identity: crate::Identity,
    pub(crate) key: Id,
    pub(crate) command: SubmitNode,
    pub(crate) part: rx_process_contract::execution_v2::executor::PartBinding,
    pub(crate) policy: rx_process_contract::execution_v2::Policy,
    pub(crate) inputs: rx_process_contract::execution_v2::InputClosure,
    pub(crate) index: rx_process_contract::execution_v2::ValidatedIndex,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
}
pub struct PreparedOperation {
    pub(crate) ticket: OperationTicket,
    pub(crate) materialized: rx_process_contract::execution_v2::Materialized,
}
impl PreparedOperation {
    pub fn prepare(ticket: OperationTicket) -> Result<Self, String> {
        let materialized = rx_process_contract::execution_v2::materialize(
            &ticket.policy,
            &ticket.inputs,
            ticket.part.candidate,
            ticket.part.slot,
        )?;
        materialized.verify_index(
            &ticket.policy,
            &ticket.index,
            ticket.part.candidate,
            ticket.part.slot,
        )?;
        if materialized.report_digest() != ticket.part.report.sha256 {
            return Err("operation report differs from immutable Part".into());
        }
        Ok(Self {
            ticket,
            materialized,
        })
    }
}
