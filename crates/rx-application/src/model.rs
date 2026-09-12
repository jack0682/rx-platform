use rx_domain::{budget::*, condition::Condition, intent::Intent, operation::Operation, types::*};
pub use rx_process_contract::checkpoint_change::*;
pub use rx_process_contract::execution::{
    ExecutionSnapshot, PartDisposition, ProcessCheckpoint, Purpose, Run, RunState, WaitWindow,
};
pub use rx_process_contract::production::{
    CompletePart as ProductionCompletePart, View as ProductionView,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub trait Clock: Send {
    fn now(&self) -> TimePoint;
}

/// Produced only by a credential-verifying adapter. Never deserialize this from an API body.
#[derive(Clone, Debug)]
pub struct Identity {
    pub principal: Name,
    pub session: Id,
    pub terminal: Option<(Name, Digest)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    Observer,
    Operator,
    RecoveryLead,
    Engineer,
    Verifier,
    ReleaseManager,
    AccountAdmin,
    Executor,
    Host,
    OperatorApi,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: Name,
    pub client_namespace: Name,
    pub roles: BTreeSet<Role>,
    pub cells: BTreeSet<Name>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalBinding>,
    pub id: Id,
    pub principal: Name,
    pub runtime_boot: Id,
    pub expires_at: TimePoint,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceProducer {
    pub principal: Name,
    pub session: Id,
    pub peer_boot: Id,
    pub journal: Id,
    pub authentication_binding: Digest,
    pub cells: BTreeMap<Name, Digest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPeer {
    pub principal: Name,
    pub session: Id,
    pub peer_boot: Id,
    pub authentication_binding: Digest,
    pub cells: BTreeMap<Name, Digest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorPeer {
    pub principal: Name,
    pub session: Id,
    pub peer_boot: Id,
    pub authentication_binding: Digest,
    pub cells: BTreeMap<Name, Digest>,
}
#[derive(Clone, Debug)]
pub enum ServicePeer {
    Host(EvidenceProducer),
    Executor(ExecutorPeer),
    OperatorApi(OperatorPeer),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalBinding {
    pub id: Name,
    pub certificate_digest: Digest,
    pub revision: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    pub id: Name,
    pub certificate_digest: Digest,
    pub cells: BTreeSet<Name>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub id: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub clock_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}

/// Resolved finite dependency steps. Source-level branch/loop compilation is separate work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepBinding {
    pub id: Name,
    pub host: Name,
    pub intent: Intent,
    pub predecessors: Vec<Name>,
    pub conditions: Vec<Condition>,
    pub completion: CompletionRule,
    pub condition_ids: Vec<Name>,
    pub condition_revision: Counter,
    pub handover_max_age_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum CompletionRule {
    Unobservable,
    NativeOutcomes {
        table: rx_process_contract::native_outcome::NativeOutcomeTable,
        postconditions: Vec<Condition>,
    },
    Native {
        schema: Name,
        success: Vec<Integer>,
        failure: Vec<Integer>,
        postconditions: Vec<Condition>,
    },
    Predicate {
        conditions: Vec<Condition>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellConfiguration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<Box<rx_process_contract::ResolvedProcess>>,
    pub id: Name,
    pub environment: Environment,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub recipe: ArtifactRef,
    pub site_config_digest: Digest,
    pub scopes: Vec<Name>,
    pub hosts: Vec<Name>,
    pub executor: Name,
    pub maximum_budget: Counter,
    pub permit_ttl_ns: Counter,
    pub start_timeout_ns: Counter,
    pub start_conditions: Vec<Condition>,
    pub steps: Vec<StepBinding>,
    pub maintained_conditions: Vec<Condition>,
    pub fact_specs: Vec<FactSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactSpec {
    pub id: Name,
    pub host: Name,
    pub schema: Name,
    pub unit: Name,
    pub maximum_age_ns: Counter,
    pub maximum_uncertainty_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    pub id: Id,
    pub revision: Counter,
    pub envelope_digest: Digest,
    pub environment: Environment,
    pub evidence: Vec<ArtifactRef>,
    pub dependencies: Vec<Digest>,
    pub reviewed_by: Name,
}
/// Immutable in-memory verification results prepared by validator workers.
/// This check must perform no I/O and must bind the exact configuration/dependencies.
/// API callers cannot provide a PASS boolean.
pub trait QualificationAuthority: Send {
    fn verify_close_policy(
        &self,
        _configuration: &CellConfiguration,
        _reference: &ArtifactRef,
        _policy: &crate::closure::Policy,
    ) -> bool {
        false
    }
    /// Trusted release/package verifier supplies this capability; default never admits a policy.
    fn verify_procedure(
        &self,
        _configuration: &CellConfiguration,
        _reference: &ArtifactRef,
        _policy: &crate::procedure::Policy,
    ) -> bool {
        false
    }
    fn verify(
        &self,
        configuration: &CellConfiguration,
        evidence: &[ArtifactRef],
        dependencies: &[Digest],
    ) -> bool;
}
/// RX operating context; hardware selector/mode observations remain independent conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperatingMode {
    Setup,
    Automatic,
    Recovery,
    Maintenance,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Commissioning {
    NotCommissioned,
    Commissioned,
    RevalidationRequired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// None preserves the absence in older records; wire projection must not invent a mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<OperatingMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commissioning: Option<Commissioning>,
    pub configuration: CellConfiguration,
    pub epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    pub qualification: Option<Qualification>,
    pub blocks: Vec<Block>,
    pub open_cases: Vec<Id>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockReason {
    ConfigurationChange,
    OutOfService,
    ProcedureReported,
    CaseDiagnostic,
    CaseIntervention,
    RuntimeRestart,
    OperatorHold,
    ExecutorPause,
    AuthorityRevoked,
    ConditionLost,
    IntegrityConflict,
    DeviceRestart,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_revision: Option<Counter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<Id>,
    pub reason: BlockReason,
    pub latched: bool,
    pub scopes: Vec<Name>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRegistration {
    pub id: Name,
    pub session: Id,
    pub boot_id: Id,
    pub delivery_journal: Id,
    pub cell: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub source_sessions: BTreeMap<Name, Id>,
    pub grant: Grant,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: Id,
    pub fence: Counter,
    pub resources: Vec<Name>,
    pub owner: Name,
    pub valid_until: TimePoint,
    pub ttl_ms: Counter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MandateState {
    Active,
    Revoked,
    Exhausted,
    Closed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRun {
    pub cell: Name,
    pub recipe_digest: Digest,
    pub site_config_digest: Digest,
    pub expected_cell: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRun {
    pub run: Id,
    pub envelope_digest: Digest,
    pub purpose: Purpose,
    pub budget_unit: BudgetUnit,
    pub budget_limit: Counter,
    pub expected_cell: Counter,
    pub expected_run: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitWork {
    pub run: Id,
    pub activation: Id,
    pub part: Option<Id>,
    pub slot: Name,
    pub intent: Intent,
    pub expected_cell: Counter,
    pub expected_run: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorSubmitRequest {
    pub cell: Name,
    pub mandate: Id,
    pub work: SubmitWork,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mandate {
    pub id: Id,
    pub run: Id,
    pub cell: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub executor_session: Id,
    pub state: MandateState,
    pub actor: Name,
    pub attempt: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StartStatus {
    Pending,
    Arming,
    Started,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartAttempt {
    pub id: Id,
    pub run: Id,
    pub cell: Name,
    pub expected_cell_revision: Counter,
    pub expected_run_revision: Counter,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub host_boots: BTreeMap<Name, Id>,
    pub acknowledgments: BTreeMap<Name, Id>,
    pub executor_session: Id,
    pub actor: Name,
    pub status: StartStatus,
    pub mandate: Option<Id>,
    pub valid_until: TimePoint,
    pub terminal: Option<(Name, Digest)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartAttempt {
    pub id: Id,
    pub run: Id,
    pub ordinal: Counter,
    pub disposition: PartDisposition,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartSnapshot {
    pub revision: Counter,
    pub part: PartAttempt,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReceipt {
    pub operation: Id,
    pub intent_digest: Digest,
    pub operation_revision: Counter,
    pub journal: Id,
    pub sequence: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginPartRequest {
    pub cell: Name,
    pub run: Id,
    pub mandate: Id,
    pub expected_budget: Counter,
    pub expected_cell: Option<Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveActivationRequest {
    pub run: Id,
    pub node: Name,
    pub visit: Counter,
    pub expected_run: Counter,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProcessProgress {
    pub checked_at: TimePoint,
    pub admission_allowed: bool,
    pub run_revision: Counter,
    pub cell_revision: Counter,
    pub checkpoint: ProcessCheckpoint,
    pub view: rx_process_contract::frontier::ProgressView,
    pub frontier: rx_process_contract::frontier::Frontier,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub id: Id,
    pub run: Id,
    pub part: Option<Id>,
    pub node: Name,
    pub visit: Counter,
    pub slots: BTreeMap<Name, Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    pub operation: Operation,
    pub intent: Intent,
    pub cell: Name,
    pub run: Id,
    pub part: Option<Id>,
    pub activation: Id,
    pub slot: Name,
    pub host: Name,
    pub permit: Id,
    pub invocation: Option<Id>,
    pub completion: CompletionRule,
    pub host_journal: Id,
    pub handover_max_age_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoverObservation {
    pub id: Id,
    pub operation: Id,
    pub invocation: Id,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub host_boot: Id,
    pub source: Name,
    pub schema: Name,
    pub value: bool,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseResources {
    pub operation: Id,
    pub expected_operation: Counter,
    pub expected_cell: Counter,
    pub observations: Vec<HandoverObservation>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PermitState {
    Issued,
    Consumed,
    Voided,
    Expired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permit {
    pub id: Id,
    pub operation: Id,
    pub intent_digest: Digest,
    pub cell: Name,
    pub mandate: Id,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub host: Name,
    pub host_boot: Id,
    pub expires_at: TimePoint,
    pub state: PermitState,
    pub grant: Grant,
    pub qualification: Id,
    pub evidence_ids: Vec<Id>,
    pub qualification_revision: Counter,
    pub envelope_digest: Digest,
    pub purpose: Purpose,
    pub issued_at: TimePoint,
    pub condition_ids: Vec<Name>,
    pub condition_revision: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub id: Name,
    pub fence: Counter,
    pub holder: Option<Id>,
    pub quarantined: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmAcknowledgment {
    pub attempt: Id,
    pub host_boot: Id,
    pub delivery_journal: Id,
    pub sequence: Counter,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Delivery {
    Arm {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clear_blocks: Vec<Id>,
        attempt: Id,
        host: Name,
        epoch: Counter,
        scopes: BTreeMap<Name, Counter>,
    },
    Prepare {
        operation: Id,
        host: Name,
        permit: Id,
    },
    Authorize {
        operation: Id,
        host: Name,
        permit: Id,
        invocation: Id,
    },
    Fence {
        cell: Name,
        host: Name,
        epoch: Counter,
        scopes: BTreeMap<Name, Counter>,
        block_ids: Vec<Id>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiptState {
    Prepared,
    SendEntered,
    NativeAccepted,
    NativeRejected,
    ResultCaptured,
    VoidedBeforeSend,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReceipt {
    pub operation: Id,
    pub digest: Digest,
    pub invocation: Option<Id>,
    pub journal: Id,
    pub sequence: Counter,
    pub state: ReceiptState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEvidence {
    pub id: Id,
    pub operation: Id,
    pub invocation: Id,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub status_schema: Name,
    pub status: Integer,
    pub captured_at: TimePoint,
    /// None is a legacy/internal projection with no claim to complete source fidelity.
    /// Every current transport decoder must supply Some, including explicit absent native fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_details: Option<NativeEvidenceDetails>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEvidenceDetails {
    pub native_id: Option<String>,
    pub native_data: Option<ArtifactRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBatch {
    pub journal: Id,
    pub first: Counter,
    pub records: Vec<NativeEvidence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCursor {
    pub journal: Id,
    pub through: Counter,
    pub disputed: bool,
}
#[derive(Clone, Debug)]
pub struct EvidenceCommit {
    pub producer: EvidenceCursor,
    pub installation: Id,
    pub store_generation: Id,
    pub platform_sequence: Counter,
}
#[derive(Clone, Debug)]
pub struct PendingDelivery {
    pub id: Id,
    pub state: rx_ports::OutboxState,
    pub payload: Delivery,
}
#[derive(Clone, Debug)]
pub struct DeliveryPlan {
    pub message: Id,
    pub first_emission: bool,
    pub cell: Name,
    pub registration: HostRegistration,
    pub payload: Delivery,
    pub work: Option<Box<Work>>,
    pub permit: Option<Box<Permit>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeliveryIssue {
    ResponseUnknown,
    RemoteNotFound,
    ReauthorizationRequired,
    PeerChanged,
    PolicyRejected,
    RemoteRejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryAttention {
    pub message: Id,
    pub host: Name,
    pub issue: DeliveryIssue,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceAcknowledgment {
    pub cell: Name,
    pub invalidation: Id,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub host_boot: Id,
    pub journal: Id,
    pub sequence: Counter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactRecord {
    pub cell: Name,
    pub id: Name,
    pub source_host: Name,
    pub source_generation: Id,
    pub schema: Name,
    pub unit: Name,
    pub acquired_at: TimePoint,
    pub maximum_age_ns: Counter,
    pub acquisition_uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
    pub disputed: bool,
    pub value: rx_domain::types::TypedValue,
    pub evidence_id: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvidence {
    pub id: Name,
    pub source_host: Name,
    pub source_generation: Id,
    pub schema: Name,
    pub unit: Name,
    pub acquired_at: TimePoint,
    pub acquisition_uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
    pub disputed: bool,
    pub value: TypedValue,
    pub evidence_id: Id,
}
impl FactRecord {
    pub fn evidence(&self) -> ObservationEvidence {
        ObservationEvidence {
            id: self.id.clone(),
            source_host: self.source_host.clone(),
            source_generation: self.source_generation.clone(),
            schema: self.schema.clone(),
            unit: self.unit.clone(),
            acquired_at: self.acquired_at.clone(),
            acquisition_uncertainty_ns: self.acquisition_uncertainty_ns,
            quality_good: self.quality_good,
            origin_age_bounded: self.origin_age_bounded,
            disputed: self.disputed,
            value: self.value.clone(),
            evidence_id: self.evidence_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseRunRequest {
    pub run: Id,
    pub expected_run: Counter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReconciliationState {
    Pending,
    Complete,
    Attention,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReconciliationIssue {
    WaitingDispatch,
    WaitingResult,
    WaitingHandover,
    SourceUnavailable,
    ContinuityUnproven,
    PermissionChanged,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationRequest {
    pub id: Id,
    pub operation: Id,
    pub cell: Name,
    pub host: Name,
    pub generation: Counter,
    pub requested_by: Name,
    pub state: ReconciliationState,
    pub issue: Option<ReconciliationIssue>,
}
#[derive(Clone, Debug)]
pub struct ReconciliationPlan {
    pub request: ReconciliationRequest,
    pub work: Work,
    pub cell_revision: Counter,
    pub receipt_message: Option<Id>,
    pub host: HostRegistration,
}
