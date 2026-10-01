//! P-owned execution intent and authenticated Supervisor facts, separate from work permission.
use crate::{
    canonical,
    component::{CatalogReference, Registration},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Effect {
    NonActuating,
    ProtocolGuardedService,
    RequiresPlatformAuthority,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPolicy {
    pub digest: Digest,
    pub effect: Effect,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub registry: Digest,
    pub releases: BTreeSet<Digest>,
    pub programs: BTreeMap<Name, CatalogPolicy>,
}
impl Enrollment {
    pub fn digest(&self, principal: &Name) -> Result<Digest, String> {
        if self.releases.is_empty()
            || self.releases.len() > 32
            || self.programs.is_empty()
            || self.programs.len() > 128
        {
            return Err("execution enrollment bounds".into());
        }
        canonical::digest("RX-SUPERVISOR-ENROLLMENT-v1", &(principal, self))
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub registry: Digest,
    pub id: Id,
    pub principal: Name,
    pub peer_boot: Id,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub authentication_binding: Digest,
    pub enrollment: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub component: Id,
    pub expected_revision: Counter,
    pub parameters: BTreeMap<Name, String>,
    pub depends_on: Vec<Name>,
    pub startup_timeout_ms: Counter,
    pub shutdown_timeout_ms: Counter,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Propose {
    pub supervisor: Name,
    pub environment: Environment,
    pub profiles: Vec<Name>,
    pub selections: BTreeMap<Name, Selection>,
}
impl Propose {
    pub fn validate(&self) -> Result<(), String> {
        if canonical::bytes(self).map_err(|e| e.to_string())?.len() > 131_072 {
            return Err("assignment payload bounds".into());
        }
        if self.selections.is_empty()
            || self.selections.len() > 32
            || self.profiles.len() > 22
            || self.profiles.iter().collect::<BTreeSet<_>>().len() != self.profiles.len()
        {
            return Err("assignment bounds".into());
        }
        let mut components = BTreeSet::new();
        for (name, s) in &self.selections {
            if !components.insert(&s.component)
                || s.expected_revision.0 == 0
                || !(100..=30_000).contains(&s.startup_timeout_ms.0)
                || !(100..=30_000).contains(&s.shutdown_timeout_ms.0)
                || s.parameters.len() > 32
                || s.parameters
                    .values()
                    .any(|v| v.is_empty() || v.len() > 256 || v.contains('\0'))
                || s.depends_on.iter().collect::<BTreeSet<_>>().len() != s.depends_on.len()
                || s.depends_on
                    .iter()
                    .any(|d| d == name || !self.selections.contains_key(d))
            {
                return Err("assignment selection shape".into());
            }
        }
        let mut done = BTreeSet::new();
        loop {
            let before = done.len();
            for (name, s) in &self.selections {
                if s.depends_on.iter().all(|d| done.contains(d)) {
                    done.insert(name.clone());
                }
            }
            if done.len() == self.selections.len() {
                return Ok(());
            }
            if before == done.len() {
                return Err("assignment dependency cycle".into());
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyOrigin {
    pub registry: Digest,
    pub freeze: crate::component_transfer::FreezeRecord,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub origin: Option<LegacyOrigin>,
    pub registration: Registration,
    pub revision: Counter,
    pub instance: Id,
    pub selection: Selection,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub id: Id,
    pub run: Id,
    pub owner: Name,
    pub supervisor: Name,
    pub environment: Environment,
    pub profiles: Vec<Name>,
    pub nodes: BTreeMap<Name, Node>,
    pub created_at: TimePoint,
}
impl Intent {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-RESIDENT-EXECUTION-INTENT-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramVerification {
    pub catalog: CatalogReference,
    pub effect: Effect,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preparation {
    pub legacy_source: Option<crate::component_transfer::FreezeRecord>,
    pub assignment: Id,
    pub intent_digest: Digest,
    pub peer: Peer,
    pub release: Digest,
    pub plan_digest: Digest,
    pub programs: BTreeMap<Name, ProgramVerification>,
}
impl Preparation {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-RESIDENT-EXECUTION-PREPARATION-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContentBasis {
    EnrolledSupervisorVerifiedRelease,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentReceipt {
    pub preparation: Preparation,
    pub basis: ContentBasis,
    pub recorded_at: TimePoint,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: Id,
    pub assignment: Id,
    pub intent_digest: Digest,
    pub preparation_digest: Digest,
    pub peer: Peer,
    pub issued_at: TimePoint,
    pub valid_until: TimePoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Proposed,
    Prepared,
    Granted,
    Running,
    StopRequested,
    Exited,
    NotStarted,
    Unknown,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObservationState {
    Assigned,
    Running,
    Exited,
    NotStarted,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservation {
    pub instance: Id,
    pub state: ObservationState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub assignment: Id,
    pub grant: Id,
    pub sequence: Counter,
    pub nodes: BTreeMap<Name, NodeObservation>,
    pub reconciliation_required: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReceipt {
    pub observer: Peer,
    pub observation: Observation,
    pub recorded_at: TimePoint,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub intent: Intent,
    pub phase: Phase,
    pub content: Option<ContentReceipt>,
    pub grant: Option<Grant>,
    pub observed: Option<ObservationReceipt>,
    pub stop_requested: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub revision: Counter,
    pub assignment: Assignment,
    pub peer_current: bool,
    pub claims_held: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approve {
    pub assignment: Id,
    pub expected_revision: Counter,
    pub preparation_digest: Digest,
    pub start_window_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stop {
    pub assignment: Id,
    pub expected_revision: Counter,
}
