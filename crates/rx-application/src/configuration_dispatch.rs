//! P-owned durable Host configuration exchange. Receipt facts and current applicability are separate.
use crate::Identity;
use rx_domain::{host_configuration as wire, types::*};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    AwaitingSnapshot,
    Prepared,
    SendEntered,
    Retired,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Issue {
    TransportUnavailable,
    ReceiptMissing,
    HostGenerationChanged,
    ObservationMismatch,
    AuthorizationChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sender {
    pub(crate) principal: Name,
    pub(crate) session: Id,
    pub(crate) terminal: Name,
    pub(crate) certificate: Digest,
}
impl Sender {
    pub(crate) fn identity(&self) -> Identity {
        Identity {
            principal: self.principal.clone(),
            session: self.session.clone(),
            terminal: Some((self.terminal.clone(), self.certificate)),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellProjection {
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
    pub before_configuration: Digest,
    pub after_configuration: Digest,
    pub recipe: ArtifactRef,
    pub required_intents: Vec<Digest>,
    pub required_conditions: Vec<Name>,
    pub epoch: Counter,
    pub scopes: std::collections::BTreeMap<Name, Counter>,
    pub fence_request: Id,
    pub change_blocks: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: Id,
    pub change: Id,
    pub origin: Name,
    pub preparation: Counter,
    pub plan_digest: Digest,
    pub host: Name,
    pub host_boot: Id,
    pub delivery_journal: Id,
    pub producer_session: Id,
    pub runtime_boot: Id,
    pub cells: Vec<CellProjection>,
    pub phase: Phase,
    pub request: Option<wire::Request>,
    pub request_digest: Option<Digest>,
    pub receipt: Option<wire::Receipt>,
    pub observation: Option<wire::Observation>,
    pub issue: Option<Issue>,
    pub integrity_disputed: bool,
    pub(crate) sender: Sender,
    pub created_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub change: Id,
    pub preparation: Counter,
    pub tasks: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Emission {
    Send { request: Box<wire::Request> },
    Lookup { request: Id },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostStatus {
    pub host: Name,
    pub task: Option<Id>,
    pub preparation: Option<Counter>,
    pub phase: Option<Phase>,
    pub outcome: Option<wire::Status>,
    pub issue: Option<Issue>,
    pub acknowledged_for_preparation: bool,
    pub integrity_disputed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub hosts: Vec<HostStatus>,
    pub mixed_configuration: bool,
    pub outcome_unknown: bool,
    pub all_hosts_acknowledged: bool,
    pub revalidation_required_before_platform_apply: bool,
    pub platform_configuration_applied: bool,
}
