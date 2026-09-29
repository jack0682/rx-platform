//! Explicit Host-change recovery. No grant, Arm or operating permission.
use crate::{
    host_binding_baseline::{HostBindingBaseline, TransportPin},
    host_invalidation::Origin,
    *,
};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Blocker {
    OriginMissing,
    OriginMismatch,
    RuntimeAlsoChanged,
    SourceIdentityChanged,
    CohortChanged,
    TransportMissing,
    BaselineMissing { cell: Name },
    BaselineMismatch { cell: Name },
    RegistrationMissing { cell: Name },
    RegistrationChanged { cell: Name },
    ContextAdvanced { cell: Name },
    ConfigurationChanged { cell: Name },
    LiveAuthority { cell: Name },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CellContext {
    pub revision: Counter,
    pub cell: Cell,
    pub registration: Option<HostRegistration>,
    pub baseline: Option<HostBindingBaseline>,
    pub baseline_digest: Option<Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Context {
    pub schema: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub clock_id: String,
    pub host: Name,
    pub origin: Name,
    pub producer_revision: Counter,
    pub producer: EvidenceProducer,
    pub replacement: Option<Origin>,
    pub replacement_digest: Option<Digest>,
    pub transport: Option<TransportPin>,
    pub cells: BTreeMap<Name, CellContext>,
    pub blockers: Vec<Blocker>,
    pub local_prerequisites_current: bool,
    pub fresh_host_read_required: bool,
    pub operation_authorized: bool,
}
impl Context {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-HOST-REJOIN-CONTEXT-v1", self).map_err(|e| e.to_string())
    }
    pub fn expected_cells(&self) -> BTreeMap<Name, Counter> {
        self.cells
            .iter()
            .map(|(name, cut)| (name.clone(), cut.revision))
            .collect()
    }
}

pub type Prepare = crate::host_recovery::Prepare;
pub const PROPOSAL_SCHEMA: &str = "rx.host-rejoin-proposal.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub schema: Name,
    pub id: Id,
    pub revision: Counter,
    pub context: Context,
    pub context_digest: Digest,
    pub read: crate::host_recovery::ReadEvidence,
    /// Older read-only proposals remain decodable with their original canonical digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_scope: Option<BTreeMap<Id, crate::host_recovery::OperationCut>>,
    pub proposed_by: crate::host_recovery::StoredActor,
    pub proposed_at: TimePoint,
    pub valid_until: TimePoint,
}
impl Proposal {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-HOST-REJOIN-PROPOSAL-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ProposalView {
    pub proposal: Proposal,
    pub proposal_digest: Digest,
    pub context_current: bool,
    pub read_current: bool,
    pub operation_authorized: bool,
}

pub type Approve = crate::host_recovery::Approve;
pub const BINDING_SCHEMA: &str = "rx.host-rejoin-binding.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub schema: Name,
    pub id: Id,
    pub revision: Counter,
    pub proposal_digest: Digest,
    pub approved_by: crate::host_recovery::StoredActor,
    pub approved_at: TimePoint,
    pub phase: crate::host_recovery::Phase,
    pub fences: BTreeMap<Name, crate::host_recovery::FenceStep>,
    pub last_read: Option<crate::host_recovery::ReadEvidence>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BindingView {
    pub binding: Binding,
    pub proposal: Proposal,
    pub context_current: bool,
    pub read_current: bool,
    pub operation_authorized: bool,
}

/// Actual restricted-worker query evidence. No deserialize implementation accepts HTTP facts.
#[derive(Clone, Debug)]
pub struct VerifiedQuery {
    pub(crate) binding: Id,
    pub(crate) operation: Id,
    pub(crate) receipt: Option<HostReceipt>,
    pub(crate) batch: Option<EvidenceBatch>,
    pub(crate) lookup: crate::host_recovery::QueryLookup,
}
impl VerifiedQuery {
    pub fn new(
        binding: Id,
        operation: Id,
        receipt: Option<HostReceipt>,
        batch: Option<EvidenceBatch>,
        lookup: crate::host_recovery::QueryLookup,
    ) -> Result<Self, String> {
        if receipt
            .as_ref()
            .is_some_and(|r| r.operation != operation || r.sequence.0 == 0)
            || batch
                .as_ref()
                .is_some_and(|b| b.first.0 == 0 || b.records.len() > 128)
            || batch.is_some()
                != matches!(lookup, crate::host_recovery::QueryLookup::PrefixObserved)
        {
            return Err("invalid actual query shape".into());
        }
        Ok(Self {
            binding,
            operation,
            receipt,
            batch,
            lookup,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryObservation {
    pub id: Id,
    pub binding: Id,
    pub operation: Id,
    pub observed_at: TimePoint,
    pub context_current_at_record: bool,
    pub result: crate::host_recovery::QueryResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementReference {
    pub binding: Id,
    pub query: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveSettlement {
    pub reference: SettlementReference,
    pub settlement: crate::settlement::Approve,
}
/// Actual pinned-worker handover observations, never a deserializable HTTP body.
#[derive(Clone, Debug)]
pub struct VerifiedHandover {
    pub(crate) read: crate::host_recovery::ReadEvidence,
    pub(crate) observations: Vec<HandoverObservation>,
}
impl VerifiedHandover {
    pub fn new(
        read: crate::host_recovery::VerifiedRead,
        observations: Vec<HandoverObservation>,
    ) -> Result<Self, String> {
        if observations.len() != 3 {
            return Err("complete handover observations required".into());
        }
        Ok(Self {
            read: read.0,
            observations,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveRebind {
    pub binding: Id,
    pub expected_binding_revision: Counter,
    pub proposal_digest: Digest,
    pub expected_cells: BTreeMap<Name, Counter>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RebindPhase {
    Approved,
    Prepared,
    Bound,
    Attention,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrantPhase {
    Pending,
    SendEntered,
    Acknowledged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RebindStep {
    pub plan: crate::host_link::Plan,
    pub phase: GrantPhase,
    pub fence: crate::FenceAcknowledgment,
    pub commit: Option<crate::host_link::Commit>,
}
pub const REBIND_SCHEMA: &str = "rx.host-rejoin-rebind.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rebind {
    pub schema: Name,
    pub id: Id,
    pub revision: Counter,
    pub binding: Id,
    pub proposal_digest: Digest,
    pub approved_by: crate::host_recovery::StoredActor,
    pub approved_at: TimePoint,
    pub valid_until: TimePoint,
    pub phase: RebindPhase,
    pub previous_fences: BTreeMap<Name, Counter>,
    pub steps: BTreeMap<Name, RebindStep>,
    pub bound_at: Option<TimePoint>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RebindView {
    pub rebind: Rebind,
    pub proposal: Proposal,
    pub current: bool,
    pub production_authorized: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationOrigin {
    pub rebind: Id,
    pub new_plan: Id,
    pub prior_plan: Id,
    pub prior_baseline_digest: Digest,
    pub configuration: Digest,
    pub applied_context: Option<rx_domain::host_configuration::AppliedContext>,
}

impl Rebind {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-HOST-REBIND-v1", self).map_err(|e| e.to_string())
    }
}
/// Historical recovery basis selected by a specific qualification review, never clearance by itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restriction {
    pub cell: Name,
    pub host: Name,
    pub block: Block,
    pub origin_session: Id,
    pub origin_digest: Digest,
    pub rebind: Id,
    pub rebind_digest: Digest,
    pub current_rebind: Id,
    pub current_rebind_digest: Digest,
    pub normal_plan: Id,
    pub baseline_digest: Digest,
    pub configuration: Digest,
}
impl Restriction {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.block.reason != BlockReason::DeviceRestart || !self.block.latched {
            return Err("Host restriction shape differs".into());
        }
        canonical::digest("RX-HOST-REBIND-RESTRICTION-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Restrictions {
    pub cell: Name,
    pub restriction_digests: BTreeMap<Id, Digest>,
    pub restrictions: Vec<Restriction>,
    pub clearance_authorized: bool,
}
