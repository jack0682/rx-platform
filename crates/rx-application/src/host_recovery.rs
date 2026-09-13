//! Explicit recovery communication. No grant, Arm or operation admission is carried here.
pub use crate::host_binding_baseline::TransportPin;
use crate::{CellConfiguration, EvidenceProducer, FenceAcknowledgment, HostReceipt, Identity};
use rx_domain::{canonical, host_configuration, host_snapshot::HostSnapshot, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "rx.host-recovery.v1";
pub const MAX_CELLS: usize = 64;
pub const MAX_OPERATIONS: usize = 128;
pub const MAX_PENDING_SCAN: usize = 4096;
pub const READ_WINDOW_NS: u64 = 100_000_000;
pub const READ_AGE_NS: u64 = 100_000_000;

/// Durable attribution only. Decoding this row authenticates nobody; each use must recheck
/// the current session, principal, terminal binding and complete cell scope through authorize.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredActor {
    pub principal: Name,
    pub session: Id,
    pub terminal: Option<(Name, Digest)>,
}
impl From<&Identity> for StoredActor {
    fn from(identity: &Identity) -> Self {
        Self {
            principal: identity.principal.clone(),
            session: identity.session.clone(),
            terminal: identity.terminal.clone(),
        }
    }
}
impl StoredActor {
    /// Restore the identifiers to be authorized, never a prior authorization result.
    pub fn identity(&self) -> Identity {
        Identity {
            principal: self.principal.clone(),
            session: self.session.clone(),
            terminal: self.terminal.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub host: Name,
    pub origin: Name,
    pub expected_context: Digest,
    pub expected_cells: BTreeMap<Name, Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approve {
    pub id: Id,
    pub expected_revision: Counter,
    pub proposal_digest: Digest,
    pub expected_cells: BTreeMap<Name, Counter>,
}

/// A persisted read is evidence data; deserializing it does not authenticate its source.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRead {
    pub snapshot: HostSnapshot,
    pub started: TimePoint,
    pub finished: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadEvidence {
    pub platform_session: Id,
    pub transport: TransportPin,
    pub configuration: host_configuration::Observation,
    pub configuration_started: TimePoint,
    pub configuration_finished: TimePoint,
    pub cells: BTreeMap<Name, SnapshotRead>,
}
/// Internal release adapter input. Never an HTTP/JSON request body or remote authentication claim.
/// The adapter must obtain every field from its actual pinned connection and local shared clock.
#[derive(Clone, Debug)]
pub struct VerifiedRead(pub(crate) ReadEvidence);
impl VerifiedRead {
    pub fn new(
        platform_session: Id,
        transport: TransportPin,
        configuration: host_configuration::Observation,
        configuration_started: TimePoint,
        configuration_finished: TimePoint,
        cells: BTreeMap<Name, SnapshotRead>,
    ) -> Result<Self, String> {
        transport.validate()?;
        configuration.validate()?;
        if cells.is_empty() || cells.len() > MAX_CELLS {
            return Err("bounded complete Host read required".into());
        }
        for (cell, read) in &cells {
            read.snapshot.validate().map_err(|e| e.to_string())?;
            if &read.snapshot.cell != cell {
                return Err("Host read cell differs".into());
            }
        }
        Ok(Self(ReadEvidence {
            platform_session,
            transport,
            configuration,
            configuration_started,
            configuration_finished,
            cells,
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Blocker {
    BaselineMissing { cell: Name },
    RegistrationMissing { cell: Name },
    IdentityChanged { cell: Name },
    RestartOriginMissing { cell: Name },
    LiveAuthority { cell: Name },
    TooManyOperations,
    FenceCandidatesAmbiguous { cell: Name },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellCut {
    pub revision: Counter,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub configuration: CellConfiguration,
    pub configuration_digest: Digest,
    pub blocks: Vec<crate::Block>,
    pub runtime_origins: BTreeMap<Id, Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationCut {
    pub revision: Counter,
    pub digest: Digest,
    pub plan: Id,
    pub plan_revision: Counter,
    pub plan_digest: Digest,
    pub baseline_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCut {
    pub operation: Id,
    pub cell: Name,
    pub permit: Id,
    pub intent_digest: Digest,
    pub profile_digest: Digest,
    pub host_journal: Id,
    pub invocation: Option<Id>,
    /// Only previously entered original messages can receive a recovered receipt.
    pub messages: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub clock_id: String,
    pub host: Name,
    pub origin: Name,
    pub transport: TransportPin,
    pub producer: EvidenceProducer,
    pub producer_revision: Counter,
    pub anchor: Option<Id>,
    pub previous_runtime_boot: Id,
    pub host_cells: Vec<Name>,
    pub cells: BTreeMap<Name, CellCut>,
    pub registrations: BTreeMap<Name, RegistrationCut>,
    pub operations: BTreeMap<Id, OperationCut>,
    pub blockers: Vec<Blocker>,
}
impl Context {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-HOST-RECOVERY-CONTEXT-v1", self).map_err(|e| e.to_string())
    }
    pub fn expected_cells(&self) -> BTreeMap<Name, Counter> {
        self.cells
            .iter()
            .map(|(id, c)| (id.clone(), c.revision))
            .collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Proposed,
    Fencing,
    RecoveryOnly,
    Attention,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FencePhase {
    Pending,
    SendEntered,
    Acknowledged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceTask {
    pub binding: Id,
    pub cell: Name,
    pub request: Id,
    pub originating_message: Option<Id>,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub block_ids: Vec<Id>,
}
impl FenceTask {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-HOST-RECOVERY-FENCE-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceStep {
    pub task: FenceTask,
    pub payload_digest: Digest,
    pub phase: FencePhase,
    pub acknowledgment: Option<FenceAcknowledgment>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub schema: Name,
    pub id: Id,
    pub revision: Counter,
    pub phase: Phase,
    /// Immutable correlation to the caller's original context. The effective context below
    /// may additionally contain blockers discovered while preparing this proposal.
    pub requested_context_digest: Digest,
    pub context: Context,
    pub proposed_by: StoredActor,
    pub proposed_at: TimePoint,
    pub proposal_read: ReadEvidence,
    pub approved_by: Option<StoredActor>,
    pub approved_at: Option<TimePoint>,
    pub fences: BTreeMap<Name, FenceStep>,
    pub last_read: Option<ReadEvidence>,
    pub detail: Option<String>,
}
impl Binding {
    pub fn proposal_digest(&self) -> Result<Digest, String> {
        canonical::digest(
            "RX-HOST-RECOVERY-PROPOSAL-v1",
            &(
                &self.id,
                self.requested_context_digest,
                &self.context,
                &self.proposed_by,
                &self.proposed_at,
                &self.proposal_read,
                self.fences
                    .iter()
                    .map(|(cell, step)| (cell, &step.task, step.payload_digest))
                    .collect::<Vec<_>>(),
            ),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub binding: Binding,
    pub current: bool,
    pub operation_authorized: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryPage {
    pub items: Vec<View>,
    pub next: Option<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryPlan {
    pub binding: Id,
    pub host: Name,
    pub producer_session: Id,
    pub platform_session: Id,
    pub operation: OperationCut,
    pub original_receipt: Option<HostReceipt>,
    /// Lookup may collect the original result; this never permits a native replay.
    pub lookup_allowed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryLookup {
    NotNeeded,
    PrefixObserved,
    Unavailable,
    Unsupported,
}
/// A filtered diagnostic view is never an ingestible producer sequence or an absence proof.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryResult {
    pub binding: Id,
    pub operation: Id,
    pub receipt: Option<HostReceipt>,
    pub lookup: QueryLookup,
    pub evidence: Vec<crate::NativeEvidence>,
    pub evidence_complete: bool,
    pub publication_required: bool,
    pub operation_authorized: bool,
}
