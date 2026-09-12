//! P-owned issuance, durable Host exchange and explicit global qualification activation.
use crate::{
    Identity, Qualification,
    configuration_dispatch::{Issue, Phase, Sender},
    package_intake::Registration,
    requalification as q,
};
use rx_domain::{host_qualification as host, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueRequest {
    pub review: Id,
    pub cell: Name,
    pub report_revision: Counter,
    pub report_digest: Digest,
    pub decision_revision: Counter,
    pub expected_cells: BTreeMap<Name, Counter>,
    pub clear_blocks: BTreeMap<Name, Vec<Id>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finalize {
    pub batch: Id,
    pub cell: Name,
    pub expected: Counter,
    pub expected_cells: BTreeMap<Name, Counter>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Pending,
    Active,
    Suspended,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuedCell {
    pub cell: Name,
    pub configuration: ArtifactRef,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub expected_revision: Counter,
    pub clear_blocks: Vec<Id>,
    pub purposes: BTreeSet<Name>,
    pub qualification: Qualification,
    pub limitations: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Batch {
    pub issuance: IssueRequest,
    pub id: Id,
    pub origin: Name,
    pub change: Id,
    pub revision: Counter,
    pub state: State,
    pub job: q::Job,
    pub report_revision: Counter,
    pub report_digest: Digest,
    pub decision_revision: Counter,
    pub registration: Registration,
    pub cells: Vec<IssuedCell>,
    pub tasks: Vec<Id>,
    pub issued_by: Name,
    pub issued_at: TimePoint,
    pub runtime_boot: Id,
    pub activated_at: Option<TimePoint>,
    pub suspended_reason: Option<Name>,
    pub(crate) sender: Sender,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: Id,
    pub batch: Id,
    pub host: Name,
    pub host_boot: Id,
    pub journal: Id,
    pub session: Id,
    pub cells: Vec<Name>,
    pub phase: Phase,
    pub request: Option<host::Request>,
    pub digest: Option<Digest>,
    pub receipt: Option<host::Receipt>,
    pub observation: Option<host::Observation>,
    pub issue: Option<Issue>,
    pub disputed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Emission {
    Send { request: Box<host::Request> },
    Lookup { request: Id },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub batch: Batch,
    pub hosts: Vec<Task>,
    pub accepted_hosts: usize,
    pub mixed: bool,
    pub outcome_unknown: bool,
    pub current: bool,
    pub operation_authorized: bool,
}
pub enum Action {
    Issue(IssueRequest),
    Activate(Finalize),
}
pub struct Ticket {
    pub(crate) action: Action,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) job: q::Job,
    pub(crate) version: q::Version,
    pub(crate) decision: q::Decision,
    pub(crate) blobs: BTreeMap<Digest, Vec<u8>>,
    pub(crate) registration: Registration,
    pub(crate) package: rx_package::store::ObjectId,
    pub(crate) issued: TimePoint,
}
impl Ticket {
    pub fn package(&self) -> &rx_package::store::ObjectId {
        &self.package
    }
    pub fn registration(&self) -> &Registration {
        &self.registration
    }
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
}
impl Prepared {
    pub fn verify(
        ticket: Ticket,
        policy: &q::Policy,
        package: rx_package::store::StoredPackage,
    ) -> Result<Self, String> {
        if package.owner() != &ticket.registration.store_owner
            || package.policy_fingerprint() != ticket.registration.policy_fingerprint
            || package.object() != &ticket.package
        {
            return Err("qualification source package proof differs".into());
        }
        let proof = q::Verified::check(
            &ticket.job,
            policy,
            ticket.version.report.clone(),
            ticket.version.signature.clone(),
            ticket.blobs.clone(),
        )?;
        if !proof.ready
            || policy.schema.as_str() != "rx.requalification-policy.v2"
            || ticket
                .job
                .request
                .cells
                .iter()
                .any(|c| c.profile.purposes.is_empty())
        {
            return Err("activation requires reviewed v2 purpose constraints".into());
        }
        Ok(Self { ticket })
    }
}
pub enum Preflight {
    Recorded(Box<Batch>),
    Verify(Box<Ticket>),
}
