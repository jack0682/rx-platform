//! Internal typed writer messages. These are not wire DTOs and do not accept caller roles.
use crate::writer::{Priority, Processor, Writer, WriterError};
use rx_application::{
    projection::{Overview, UserProfile},
    *,
};
use rx_domain::types::*;
use rx_ports::{Repository, StoreError};

pub enum Command {
    PrepareQualificationIssue {
        identity: Identity,
        key: Id,
        input: rx_application::qualification_activation::IssueRequest,
    },
    CommitQualificationIssue(Box<rx_application::qualification_activation::Prepared>),
    PrepareQualificationActivation {
        identity: Identity,
        key: Id,
        input: rx_application::qualification_activation::Finalize,
    },
    CommitQualificationActivation(Box<rx_application::qualification_activation::Prepared>),
    SuspendQualification {
        identity: Identity,
        key: Id,
        batch: Id,
        reason: Name,
    },
    GetQualificationBatch {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    GetQualificationTasks {
        identity: Identity,
        after: Option<Id>,
    },
    BindQualificationRequest {
        identity: Identity,
        task: Id,
        observation: Box<rx_domain::host_qualification::Observation>,
        read_started: TimePoint,
    },
    EnterQualificationSend {
        identity: Identity,
        task: Id,
        retry_missing: bool,
    },
    QualificationTaskIssue {
        identity: Identity,
        task: Id,
        issue: configuration_dispatch::Issue,
    },
    RecordQualificationObservation {
        identity: Identity,
        task: Id,
        observation: Box<rx_domain::host_qualification::Observation>,
        read_started: TimePoint,
    },

    ConfigureRequalification(Option<rx_application::requalification::Policy>),
    BeginRequalification {
        identity: Identity,
        key: Id,
        input: rx_application::requalification::Begin,
    },
    PrepareRequalificationReport {
        identity: Identity,
        key: Id,
        input: rx_application::requalification::Submit,
    },
    CommitRequalificationReport(Box<rx_application::requalification::Prepared>),
    PrepareRequalificationDecision {
        identity: Identity,
        key: Id,
        input: rx_application::requalification::Decide,
    },
    CommitRequalificationDecision(Box<rx_application::requalification::PreparedDecision>),
    GetRequalification {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    GetRequalificationArtifact {
        identity: Identity,
        cell: Name,
        id: Id,
        reference: ArtifactRef,
    },

    AuthorizeHostConfiguration {
        identity: Identity,
        key: Id,
        input: process_change::Transition,
    },
    HostConfigurationTasks {
        identity: Identity,
        after: Option<Id>,
    },
    BindHostConfiguration {
        identity: Identity,
        task: Id,
        observation: Box<rx_domain::host_configuration::Observation>,
        read_started: TimePoint,
    },
    EnterHostConfigurationSend {
        identity: Identity,
        task: Id,
        retry_missing: bool,
    },
    HostConfigurationIssue {
        identity: Identity,
        task: Id,
        issue: configuration_dispatch::Issue,
    },
    RecordHostConfiguration {
        identity: Identity,
        task: Id,
        read_started: TimePoint,
        observation: Box<rx_domain::host_configuration::Observation>,
    },
    PrepareProcessChange {
        identity: Identity,
        key: Id,
        input: process_change::Create,
    },
    CommitProcessChange(Box<process_change::Prepared>),
    ReviewProcessChangeImpact {
        identity: Identity,
        key: Id,
        input: process_change::ReviewImpact,
    },
    PrepareProcessChangeApply {
        identity: Identity,
        key: Id,
        input: process_change::Transition,
    },
    CommitProcessChangeApply(Box<process_change::Prepared>),
    PrepareProcessChangeStage {
        identity: Identity,
        key: Id,
        input: process_change::Transition,
    },
    CommitProcessChangeStage(Box<process_change::Prepared>),
    BeginProcessChangePreparation {
        identity: Identity,
        key: Id,
        input: process_change::BeginPreparation,
    },
    GetProcessChange {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    ListProcessReviews {
        identity: Identity,
        cell: Name,
        intake: Id,
        after: Option<Id>,
    },
    GetProcessReviewRevision {
        identity: Identity,
        cell: Name,
        id: Id,
        revision: Option<Counter>,
    },
    ConfigureProcessReview(Option<Digest>),
    CreateProcessReview {
        identity: Identity,
        key: Id,
        input: process_review::Create,
    },
    PrepareReviewReport {
        identity: Identity,
        key: Id,
        input: process_review::Submit,
    },
    CommitReviewReport(Box<process_review::Prepared>),
    GetProcessReview {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    PrepareReviewDecision {
        identity: Identity,
        key: Id,
        input: process_review::Decide,
    },
    CommitReviewDecision(Box<process_review::PreparedDecision>),
    ConfigurePackageIntake(Option<(Id, Digest, Digest)>),
    ConfigureDeviceReview(Option<Digest>),
    PrepareDeviceBinding {
        identity: Identity,
        key: Id,
        input: rx_application::device_binding::Propose,
    },
    CommitDeviceBinding(Box<rx_application::device_binding::Prepared>),
    PrepareDeviceBindingReview {
        identity: Identity,
        key: Id,
        input: rx_application::device_binding::ReviewImpact,
    },
    CommitDeviceBindingReview(Box<rx_application::device_binding::Prepared>),
    GetDeviceBindingPlan {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    ListDeviceBindingPlans {
        identity: Identity,
        cell: Name,
        after: Option<Id>,
    },
    CreateDeviceReview {
        identity: Identity,
        key: Id,
        input: rx_application::device_review::Create,
    },
    PrepareDeviceReport {
        identity: Identity,
        key: Id,
        input: rx_application::device_review::Submit,
    },
    CommitDeviceReport(Box<rx_application::device_review::Prepared>),
    PrepareDeviceDecision {
        identity: Identity,
        key: Id,
        input: rx_application::device_review::Decide,
    },
    CommitDeviceDecision(Box<rx_application::device_review::PreparedDecision>),
    GetDeviceReview {
        identity: Identity,
        cell: Name,
        id: Id,
        revision: Option<Counter>,
    },
    ListDeviceReviews {
        identity: Identity,
        cell: Name,
        intake: Id,
        after: Option<Id>,
    },
    PackageIntakeContext {
        identity: Identity,
        cell: Name,
    },
    PreparePackageIntake {
        identity: Identity,
        key: Id,
        input: package_intake::Submit,
    },
    CommitPackageIntake(Box<package_intake::Prepared>),
    GetPackageIntake {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    GetPackageDeviceCatalog {
        identity: Identity,
        cell: Name,
        id: Id,
    },
    ListPackageIntakes {
        identity: Identity,
        cell: Name,
        after: Option<Id>,
    },
    ExportDraftCompileInput {
        identity: Identity,
        cell: Name,
        id: Id,
        source_revision: Counter,
        binding_revision: Counter,
    },
    DraftBindingCatalog {
        identity: Identity,
        cell: Name,
    },
    DraftDeviceBindingCatalog {
        identity: Identity,
        input: rx_application::draft_bindings::SelectCatalog,
    },
    SaveDraftBindings {
        identity: Identity,
        key: Id,
        input: rx_application::draft_bindings::Save,
    },
    GetDraftBindings {
        identity: Identity,
        cell: Name,
        id: Id,
        revision: Option<Counter>,
    },
    SaveProcessDraft {
        identity: Identity,
        key: Id,
        prepared: rx_application::process_draft::PreparedSave,
    },
    GetProcessDraft {
        identity: Identity,
        cell: Name,
        id: Id,
        revision: Option<Counter>,
    },
    ListProcessDrafts {
        identity: Identity,
        cell: Name,
        after: Option<Id>,
    },
    ConfigureHostServices(Vec<rx_application::service_health::Target>),
    ReplaceHostService(rx_application::service_health::Owner),
    PublishHostService(Box<rx_application::service_health::Publish>),
    IngestHostRead(Box<rx_application::observation::HostRead>),
    CheckMaintainedConditions,
    ReportFact {
        identity: Identity,
        fact: FactRecord,
    },
    PrepareHostLink(Box<rx_application::host_link::Prepare>),
    CommitHostLink(Box<rx_application::host_link::Commit>),
    CurrentTime,
    BoundHostLink(Id),
    PrepareHostRenewal(Id),
    CommitHostRenewal {
        renewal: Box<rx_application::host_link::Renewal>,
        grant: Grant,
    },
    RequestRuntimeStop,
    CommitRuntimeProcessStop,
    RuntimeLifecycle,
    PutTerminal {
        identity: Identity,
        terminal: Terminal,
        expected: Option<Counter>,
    },
    OpenTerminalUserSession {
        principal: Name,
        id: Id,
        ttl_ns: Counter,
        certificate: Digest,
    },
    OpenOperatorPeer {
        principal: Name,
        peer_boot: Id,
        authentication_binding: Digest,
    },
    NegotiateOperatorCell {
        identity: Identity,
        definition: Digest,
    },
    InspectPeerCell {
        identity: Identity,
        cell: Name,
    },
    AdmitClosePolicy {
        identity: Identity,
        policy: rx_application::closure::Policy,
        reference: ArtifactRef,
    },
    PrepareClose {
        identity: Identity,
        key: Id,
        command: rx_application::closure::PrepareClose,
    },
    CloseWithoutRestart {
        identity: Identity,
        key: Id,
        command: rx_application::closure::CloseWithoutRestart,
    },
    AdmitProcedure {
        identity: Identity,
        policy: rx_application::procedure::Policy,
        reference: ArtifactRef,
    },
    RecordProcedure {
        identity: Identity,
        key: Id,
        command: Box<rx_application::procedure::Submission>,
    },
    OpenCase {
        identity: Identity,
        key: Id,
        command: rx_application::intervention::OpenCase,
    },
    Cases {
        identity: Identity,
        cell: Name,
    },
    InspectCase {
        identity: Identity,
        cell: Name,
        case: Id,
    },
    AcknowledgeCase {
        identity: Identity,
        key: Id,
        command: rx_application::intervention::AcknowledgeCase,
    },
    ProductionView {
        identity: Identity,
        run: Id,
    },
    ExecutorCompletePart {
        identity: Identity,
        key: Id,
        command: ProductionCompletePart,
    },
    PrepareCheckpoint {
        identity: Identity,
        target: CheckpointTarget,
    },
    CommitCheckpoint {
        identity: Identity,
        key: Id,
        command: Box<CommitCheckpoint>,
    },
    RecordReconciliationObservations {
        identity: Identity,
        operation: Id,
        request: Id,
        observations: Vec<HandoverObservation>,
    },
    RequestReconciliation {
        identity: Identity,
        operation: Id,
    },
    PendingReconciliations {
        identity: Identity,
        after: Option<Id>,
        limit: usize,
    },
    PlanReconciliation {
        identity: Identity,
        operation: Id,
        request: Id,
    },
    UpdateReconciliation {
        identity: Identity,
        operation: Id,
        request: Id,
        state: ReconciliationState,
        issue: Option<ReconciliationIssue>,
    },
    PauseExecutorRun {
        identity: Identity,
        key: Id,
        command: PauseRunRequest,
    },
    ExecutionSnapshot {
        identity: Identity,
        run: Id,
        visit: Counter,
    },
    ExecutorArtifact {
        identity: Identity,
        run: Id,
        reference: ArtifactRef,
    },
    ExecutorSubmit {
        identity: Identity,
        key: Id,
        command: Box<ExecutorSubmitRequest>,
    },
    ExecutorWork {
        identity: Identity,
        operation: Id,
    },
    ExecutorBeginPart {
        identity: Identity,
        key: Id,
        command: BeginPartRequest,
    },
    ExecutorResolveActivation {
        identity: Identity,
        key: Id,
        command: ResolveActivationRequest,
    },
    OpenExecutorPeer {
        principal: Name,
        peer_boot: Id,
        authentication_binding: Digest,
    },
    InspectServicePeer(Identity),
    NegotiateExecutorCell {
        identity: Identity,
        definition: Digest,
    },
    ExecutorRunCheckpoint {
        identity: Identity,
        run: Id,
    },
    RunCheckpoint {
        identity: Identity,
        run: Id,
    },
    CheckpointArtifact {
        identity: Identity,
        run: Id,
        reference: ArtifactRef,
    },
    ProcessProgress {
        identity: Identity,
        run: Id,
        visit: Counter,
    },
    ChooseBranch {
        identity: Identity,
        key: Id,
        run: Id,
        node: Name,
        visit: Counter,
        expected_run: Counter,
    },
    StartProcessWait {
        identity: Identity,
        key: Id,
        run: Id,
        node: Name,
        visit: Counter,
        expected_run: Counter,
    },
    CheckProcessWait {
        identity: Identity,
        run: Id,
        node: Name,
        visit: Counter,
        expected_run: Counter,
    },
    PendingDeliveries {
        after: Option<Id>,
        limit: usize,
    },
    PlanDelivery {
        identity: Identity,
        message: Id,
    },
    DeliveryAttention {
        message: Id,
        issue: DeliveryIssue,
    },
    RecordReceipt {
        identity: Identity,
        message: Id,
        receipt: HostReceipt,
    },
    FinishArm {
        identity: Identity,
        message: Id,
        ack: ArmAcknowledgment,
    },
    FinishFence {
        identity: Identity,
        message: Id,
        ack: FenceAcknowledgment,
    },
    ReconciliationWork {
        identity: Identity,
        after: Option<Id>,
        limit: usize,
    },
    InspectWork {
        identity: Identity,
        operation: Id,
    },
    InspectRun {
        identity: Identity,
        run: Id,
    },
    RuntimeRestrictions {
        identity: Identity,
        cell: Name,
    },
    BeginPart {
        identity: Identity,
        key: Id,
        run: Id,
        expected_budget: Counter,
    },
    ResolveActivation {
        identity: Identity,
        run: Id,
        node: Name,
        visit: Counter,
        expected_run: Counter,
    },
    SubmitWork {
        identity: Identity,
        key: Id,
        command: Box<SubmitWork>,
    },
    ReleaseResources {
        identity: Identity,
        key: Id,
        command: ReleaseResources,
    },
    CompletePart {
        identity: Identity,
        key: Id,
        part: Id,
        expected_run: Counter,
    },
    OpenEvidenceProducer {
        principal: Name,
        peer_boot: Id,
        journal: Id,
        authentication_binding: Digest,
    },
    NegotiateEvidenceCell {
        identity: Identity,
        definition: Digest,
    },
    InspectEvidenceProducer(Identity),
    CurrentEvidenceProducer(Name),
    IngestEvidence {
        identity: Identity,
        batch: EvidenceBatch,
    },
    PublishEvidence {
        identity: Identity,
        batch: EvidenceBatch,
    },
    GetNativeEvidence {
        identity: Identity,
        evidence: Id,
    },
    Installation,
    OpenUserSession {
        principal: Name,
        id: Id,
        ttl_ns: Counter,
    },
    UserProfile(Identity),
    EndUserSession(Identity),
    Overview(Identity),
    InspectCell {
        identity: Identity,
        cell: Name,
    },
    InstallCell {
        identity: Identity,
        key: Id,
        configuration: Box<CellConfiguration>,
    },
    PutPrincipal {
        identity: Identity,
        value: Principal,
        expected: Option<Counter>,
    },
    CreateRun {
        identity: Identity,
        key: Id,
        command: CreateRun,
    },
    StartRun {
        identity: Identity,
        key: Id,
        command: StartRun,
    },
    Hold {
        identity: Identity,
        key: Id,
        cell: Name,
    },
}
pub enum Reply {
    QualificationPreflight(rx_application::qualification_activation::Preflight),
    QualificationBatch(Box<rx_application::qualification_activation::Batch>),
    QualificationView(Box<rx_application::qualification_activation::View>),
    QualificationTasks(Vec<rx_application::qualification_activation::Task>),
    QualificationTask(Box<rx_application::qualification_activation::Task>),
    QualificationEmission(rx_application::qualification_activation::Emission),

    RequalificationJob(Box<rx_application::requalification::Job>),
    RequalificationPreflight(rx_application::requalification::Preflight),
    RequalificationVersion(Box<rx_application::requalification::Version>),
    RequalificationDecisionPreflight(rx_application::requalification::DecisionPreflight),
    RequalificationDecision(Box<rx_application::requalification::Decision>),
    RequalificationDetail(Box<rx_application::requalification::Detail>),

    HostConfigurationBatch(configuration_dispatch::Batch),
    HostConfigurationTasks(Vec<configuration_dispatch::Task>),
    HostConfigurationTask(Box<configuration_dispatch::Task>),
    HostConfigurationEmission(configuration_dispatch::Emission),
    ProcessChangePreflight(process_change::Preflight),
    ProcessChange(Box<process_change::Change>),
    ProcessChangeDetail(Box<process_change::Detail>),
    ProcessReviews(Box<process_review::Page>),
    ProcessReviewConfigured,
    ProcessReviewJob(Box<process_review::Job>),
    ReviewPreflight(process_review::Preflight),
    ProcessReviewVersion(Box<process_review::Version>),
    ProcessReviewDetail(Box<process_review::Detail>),
    ReviewDecisionPreflight(process_review::DecisionPreflight),
    ProcessReviewDecision(Box<process_review::Decision>),
    PackageIntakeRegistration(Option<package_intake::Registration>),
    DeviceReviewConfigured,
    DeviceBindingPreflight(rx_application::device_binding::Preflight),
    DeviceBindingPlan(Box<rx_application::device_binding::Plan>),
    DeviceBindingDetail(Box<rx_application::device_binding::Detail>),
    DeviceBindingPlans(Box<rx_application::device_binding::Page>),
    DeviceReviewJob(Box<rx_application::device_review::Job>),
    DeviceReportPreflight(rx_application::device_review::Preflight),
    DeviceReviewVersion(Box<rx_application::device_review::Version>),
    DeviceDecisionPreflight(rx_application::device_review::DecisionPreflight),
    DeviceReviewDecision(Box<rx_application::device_review::Decision>),
    DeviceReviewDetail(Box<rx_application::device_review::Detail>),
    DeviceReviews(Box<rx_application::device_review::Page>),
    PackageIntakeContext(Box<package_intake::Context>),
    PackageIntakePreflight(package_intake::Preflight),
    PackageIntakeReceipt(Box<package_intake::Receipt>),
    PackageIntakeView(Box<package_intake::View>),
    PackageDeviceCatalog(Box<rx_application::device_catalog::Detail>),
    PackageIntakes(Box<package_intake::Page>),
    DraftCompileInput(Box<rx_process_contract::compile_input::CompileInput>),
    DraftBindingCatalog(Box<rx_application::draft_bindings::Catalog>),
    DraftBindingVersion(Box<rx_application::draft_bindings::Version>),
    DraftBindingView(Box<rx_application::draft_bindings::View>),
    ProcessDraft(Box<rx_application::process_draft::Detail>),
    ProcessDrafts(Box<rx_application::process_draft::Page>),
    HostServiceOwners(Vec<rx_application::service_health::Owner>),
    HostServiceOwner(Box<rx_application::service_health::Owner>),
    Observations(rx_application::observation::BatchReceipt),
    MaintainedRevoked(Vec<Name>),
    HostLink(Box<rx_application::host_link::Plan>),
    HostRegistration(Box<HostRegistration>),
    Time(TimePoint),
    HostRenewal(Box<rx_application::host_link::Renewal>),
    RuntimeStop(Box<rx_application::lifecycle::StopReport>),
    RuntimeLifecycle(rx_application::lifecycle::Lifecycle),
    Clearance(Box<rx_application::closure::Clearance>),
    CloseReceipt(Box<rx_application::closure::Receipt>),
    ProcedureReceipt(Box<rx_application::procedure::Receipt>),
    CaseSnapshot(Box<rx_application::intervention::CaseSnapshot>),
    CaseDetail(Box<rx_application::intervention::CaseDetail>),
    Cases(Box<rx_application::intervention::CaseList>),
    ProductionView(Box<ProductionView>),
    CheckpointPreparation(Box<CheckpointPreparation>),
    ReconciliationRequests(Vec<ReconciliationRequest>),
    ReconciliationPlan(Box<ReconciliationPlan>),
    ExecutionSnapshot(Box<ExecutionSnapshot>),
    AdmissionReceipt(AdmissionReceipt),
    PartSnapshot(PartSnapshot),
    ActivationSnapshot(rx_application::checkpoint_artifact::ActivationSnapshot),
    ServicePeer(ServicePeer),
    RunCheckpoint(Box<rx_application::checkpoint_artifact::RunSnapshot>),
    ArtifactBytes(Vec<u8>),
    ProcessProgress(Box<ProcessProgress>),
    BranchChoice(rx_process_contract::frontier::BranchChoice),
    WaitWindow(WaitWindow),
    WaitProgress(Option<rx_process_contract::frontier::WaitProgress>),
    PendingDeliveries(Vec<PendingDelivery>),
    DeliveryPlan(Box<DeliveryPlan>),
    Work(Box<Work>),
    ReconciliationWork(Vec<Work>),
    VersionedRun(Counter, Run),
    RuntimeRestrictions(Box<rx_application::runtime_invalidation::RuntimeRestrictions>),
    Part(PartAttempt),
    Activation(Activation),
    Producer(EvidenceProducer),
    EvidenceCommit(EvidenceCommit),
    NativeEvidence(NativeEvidence),
    CellName(Name),
    Installation(Installation),
    Session(Session),
    Profile(UserProfile),
    Overview(Box<Overview>),
    Cell(Counter, Box<Cell>),
    Run(Run),
    Attempt(StartAttempt),
    Held(Box<Cell>),
    Done,
    Revision(Counter),
}

pub struct Application<R, C, A> {
    engine: Engine<R, C, A>,
    service_health: crate::service_health::Registry,
}
impl<R, C, A> Application<R, C, A> {
    pub fn new(engine: Engine<R, C, A>) -> Self {
        Self {
            engine,
            service_health: crate::service_health::Registry::default(),
        }
    }
}
impl<R: Repository + Send + 'static, C: Clock + 'static, A: QualificationAuthority + 'static>
    Processor for Application<R, C, A>
{
    type Command = Command;
    type Reply = Reply;
    type Error = StoreError;
    fn priority(command: &Command) -> Priority {
        match command {
            Command::SuspendQualification { .. }
            | Command::BeginProcessChangePreparation { .. }
            | Command::BeginRequalification { .. }
            | Command::ConfigureRequalification(_)
            | Command::ConfigureProcessReview(_)
            | Command::ConfigurePackageIntake(_)
            | Command::RequestRuntimeStop
            | Command::CheckMaintainedConditions
            | Command::CommitRuntimeProcessStop
            | Command::PutTerminal { .. }
            | Command::RecordProcedure { .. }
            | Command::OpenCase { .. }
            | Command::PauseExecutorRun { .. }
            | Command::Hold { .. }
            | Command::OpenExecutorPeer { .. }
            | Command::EndUserSession(_)
            | Command::PutPrincipal { .. }
            | Command::DeliveryAttention { .. }
            | Command::FinishFence { .. } => Priority::Control,
            _ => Priority::Normal,
        }
    }
    fn process(&mut self, command: Command) -> rx_ports::Result<Reply> {
        match command {
            Command::PrepareQualificationIssue {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_qualification_issue(&identity, &key, input)
                .map(Reply::QualificationPreflight),
            Command::CommitQualificationIssue(p) => self
                .engine
                .commit_qualification_issue(*p)
                .map(|b| Reply::QualificationBatch(Box::new(b))),
            Command::PrepareQualificationActivation {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_qualification_activation(&identity, &key, input)
                .map(Reply::QualificationPreflight),
            Command::CommitQualificationActivation(p) => self
                .engine
                .commit_qualification_activation(*p)
                .map(|b| Reply::QualificationBatch(Box::new(b))),
            Command::SuspendQualification {
                identity,
                key,
                batch,
                reason,
            } => self
                .engine
                .suspend_qualification(&identity, &key, &batch, reason)
                .map(|b| Reply::QualificationBatch(Box::new(b))),
            Command::GetQualificationBatch { identity, cell, id } => self
                .engine
                .qualification_batch(&identity, &cell, &id)
                .map(|b| Reply::QualificationView(Box::new(b))),
            Command::GetQualificationTasks { identity, after } => self
                .engine
                .qualification_tasks(&identity, after.as_ref())
                .map(Reply::QualificationTasks),
            Command::BindQualificationRequest {
                identity,
                task,
                observation,
                read_started,
            } => self
                .engine
                .bind_qualification_request(&identity, &task, *observation, read_started)
                .map(|t| Reply::QualificationTask(Box::new(t))),
            Command::EnterQualificationSend {
                identity,
                task,
                retry_missing,
            } => self
                .engine
                .enter_qualification_send(&identity, &task, retry_missing)
                .map(Reply::QualificationEmission),
            Command::QualificationTaskIssue {
                identity,
                task,
                issue,
            } => self
                .engine
                .qualification_issue(&identity, &task, issue)
                .map(|()| Reply::Done),
            Command::RecordQualificationObservation {
                identity,
                task,
                observation,
                read_started,
            } => self
                .engine
                .record_qualification_observation(&identity, &task, *observation, read_started)
                .map(|t| Reply::QualificationTask(Box::new(t))),

            Command::ConfigureRequalification(p) => self
                .engine
                .configure_requalification(p)
                .map(|()| Reply::Done),
            Command::BeginRequalification {
                identity,
                key,
                input,
            } => self
                .engine
                .begin_requalification(&identity, &key, input)
                .map(|v| Reply::RequalificationJob(Box::new(v))),
            Command::PrepareRequalificationReport {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_requalification_report(&identity, &key, input)
                .map(Reply::RequalificationPreflight),
            Command::CommitRequalificationReport(p) => self
                .engine
                .commit_requalification_report(*p)
                .map(|v| Reply::RequalificationVersion(Box::new(v))),
            Command::PrepareRequalificationDecision {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_requalification_decision(&identity, &key, input)
                .map(Reply::RequalificationDecisionPreflight),
            Command::CommitRequalificationDecision(p) => self
                .engine
                .commit_requalification_decision(*p)
                .map(|v| Reply::RequalificationDecision(Box::new(v))),
            Command::GetRequalification { identity, cell, id } => self
                .engine
                .requalification(&identity, &cell, &id)
                .map(|v| Reply::RequalificationDetail(Box::new(v))),
            Command::GetRequalificationArtifact {
                identity,
                cell,
                id,
                reference,
            } => self
                .engine
                .requalification_artifact(&identity, &cell, &id, &reference)
                .map(Reply::ArtifactBytes),

            Command::AuthorizeHostConfiguration {
                identity,
                key,
                input,
            } => self
                .engine
                .authorize_host_configuration(&identity, &key, input)
                .map(Reply::HostConfigurationBatch),
            Command::HostConfigurationTasks { identity, after } => self
                .engine
                .host_configuration_tasks(&identity, after.as_ref())
                .map(Reply::HostConfigurationTasks),
            Command::BindHostConfiguration {
                identity,
                task,
                observation,
                read_started,
            } => self
                .engine
                .bind_host_configuration(&identity, &task, *observation, read_started)
                .map(|v| Reply::HostConfigurationTask(Box::new(v))),
            Command::EnterHostConfigurationSend {
                identity,
                task,
                retry_missing,
            } => self
                .engine
                .enter_host_configuration_send(&identity, &task, retry_missing)
                .map(Reply::HostConfigurationEmission),
            Command::HostConfigurationIssue {
                identity,
                task,
                issue,
            } => self
                .engine
                .note_host_configuration_issue(&identity, &task, issue)
                .map(|()| Reply::Done),
            Command::RecordHostConfiguration {
                identity,
                task,
                read_started,
                observation,
            } => self
                .engine
                .record_host_configuration_read(&identity, &task, *observation, read_started)
                .map(|v| Reply::HostConfigurationTask(Box::new(v))),
            Command::PrepareProcessChange {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_process_change(&identity, &key, input)
                .map(Reply::ProcessChangePreflight),
            Command::CommitProcessChange(p) => self
                .engine
                .commit_process_change(*p)
                .map(|v| Reply::ProcessChange(Box::new(v))),
            Command::ReviewProcessChangeImpact {
                identity,
                key,
                input,
            } => self
                .engine
                .review_process_change_impact(&identity, &key, input)
                .map(|v| Reply::ProcessChange(Box::new(v))),
            Command::PrepareProcessChangeApply {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_process_change_apply(&identity, &key, input)
                .map(Reply::ProcessChangePreflight),
            Command::CommitProcessChangeApply(p) => self
                .engine
                .commit_process_change_apply(*p)
                .map(|c| Reply::ProcessChange(Box::new(c))),
            Command::PrepareProcessChangeStage {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_process_change_stage(&identity, &key, input)
                .map(Reply::ProcessChangePreflight),
            Command::CommitProcessChangeStage(p) => self
                .engine
                .commit_process_change_stage(*p)
                .map(|v| Reply::ProcessChange(Box::new(v))),
            Command::BeginProcessChangePreparation {
                identity,
                key,
                input,
            } => self
                .engine
                .begin_process_change_preparation(&identity, &key, input)
                .map(|v| Reply::ProcessChange(Box::new(v))),
            Command::GetProcessChange { identity, cell, id } => self
                .engine
                .process_change(&identity, &cell, &id)
                .map(|v| Reply::ProcessChangeDetail(Box::new(v))),
            Command::ListProcessReviews {
                identity,
                cell,
                intake,
                after,
            } => self
                .engine
                .process_reviews(&identity, &cell, &intake, after.as_ref())
                .map(|v| Reply::ProcessReviews(Box::new(v))),
            Command::GetProcessReviewRevision {
                identity,
                cell,
                id,
                revision,
            } => self
                .engine
                .process_review_revision(&identity, &cell, &id, revision)
                .map(|v| Reply::ProcessReviewDetail(Box::new(v))),
            Command::ConfigureProcessReview(digest) => self
                .engine
                .configure_process_review(digest)
                .map(|()| Reply::ProcessReviewConfigured),
            Command::CreateProcessReview {
                identity,
                key,
                input,
            } => self
                .engine
                .create_process_review(&identity, &key, input)
                .map(|v| Reply::ProcessReviewJob(Box::new(v))),
            Command::PrepareReviewReport {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_review_report(&identity, &key, input)
                .map(Reply::ReviewPreflight),
            Command::CommitReviewReport(prepared) => self
                .engine
                .commit_review_report(*prepared)
                .map(|v| Reply::ProcessReviewVersion(Box::new(v))),
            Command::GetProcessReview { identity, cell, id } => self
                .engine
                .process_review(&identity, &cell, &id)
                .map(|v| Reply::ProcessReviewDetail(Box::new(v))),
            Command::PrepareReviewDecision {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_review_decision(&identity, &key, input)
                .map(Reply::ReviewDecisionPreflight),
            Command::CommitReviewDecision(prepared) => self
                .engine
                .commit_review_decision(*prepared)
                .map(|v| Reply::ProcessReviewDecision(Box::new(v))),
            Command::PrepareDeviceBinding {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_device_binding(&identity, &key, input)
                .map(Reply::DeviceBindingPreflight),
            Command::CommitDeviceBinding(p) => self
                .engine
                .commit_device_binding(*p)
                .map(|v| Reply::DeviceBindingPlan(Box::new(v))),
            Command::PrepareDeviceBindingReview {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_device_binding_review(&identity, &key, input)
                .map(Reply::DeviceBindingPreflight),
            Command::CommitDeviceBindingReview(p) => self
                .engine
                .commit_device_binding_review(*p)
                .map(|v| Reply::DeviceBindingPlan(Box::new(v))),
            Command::GetDeviceBindingPlan { identity, cell, id } => self
                .engine
                .device_binding_plan(&identity, &cell, &id)
                .map(|v| Reply::DeviceBindingDetail(Box::new(v))),
            Command::ListDeviceBindingPlans {
                identity,
                cell,
                after,
            } => self
                .engine
                .device_binding_plans(&identity, &cell, after.as_ref())
                .map(|v| Reply::DeviceBindingPlans(Box::new(v))),
            Command::ConfigureDeviceReview(digest) => self
                .engine
                .configure_device_review(digest)
                .map(|_| Reply::DeviceReviewConfigured),
            Command::CreateDeviceReview {
                identity,
                key,
                input,
            } => self
                .engine
                .create_device_review(&identity, &key, input)
                .map(|v| Reply::DeviceReviewJob(Box::new(v))),
            Command::PrepareDeviceReport {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_device_report(&identity, &key, input)
                .map(Reply::DeviceReportPreflight),
            Command::CommitDeviceReport(v) => self
                .engine
                .commit_device_report(*v)
                .map(|v| Reply::DeviceReviewVersion(Box::new(v))),
            Command::PrepareDeviceDecision {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_device_decision(&identity, &key, input)
                .map(Reply::DeviceDecisionPreflight),
            Command::CommitDeviceDecision(v) => self
                .engine
                .commit_device_decision(*v)
                .map(|v| Reply::DeviceReviewDecision(Box::new(v))),
            Command::GetDeviceReview {
                identity,
                cell,
                id,
                revision,
            } => self
                .engine
                .device_review(&identity, &cell, &id, revision)
                .map(|v| Reply::DeviceReviewDetail(Box::new(v))),
            Command::ListDeviceReviews {
                identity,
                cell,
                intake,
                after,
            } => self
                .engine
                .device_reviews(&identity, &cell, &intake, after.as_ref())
                .map(|v| Reply::DeviceReviews(Box::new(v))),
            Command::ConfigurePackageIntake(configuration) => self
                .engine
                .configure_package_intake(configuration)
                .map(Reply::PackageIntakeRegistration),
            Command::PackageIntakeContext { identity, cell } => self
                .engine
                .package_intake_context(&identity, &cell)
                .map(|v| Reply::PackageIntakeContext(Box::new(v))),
            Command::PreparePackageIntake {
                identity,
                key,
                input,
            } => self
                .engine
                .prepare_package_intake(&identity, &key, input)
                .map(Reply::PackageIntakePreflight),
            Command::CommitPackageIntake(prepared) => self
                .engine
                .commit_package_intake(*prepared)
                .map(|v| Reply::PackageIntakeReceipt(Box::new(v))),
            Command::GetPackageIntake { identity, cell, id } => self
                .engine
                .package_intake(&identity, &cell, &id)
                .map(|v| Reply::PackageIntakeView(Box::new(v))),
            Command::GetPackageDeviceCatalog { identity, cell, id } => self
                .engine
                .package_device_catalog(&identity, &cell, &id)
                .map(|v| Reply::PackageDeviceCatalog(Box::new(v))),
            Command::ListPackageIntakes {
                identity,
                cell,
                after,
            } => self
                .engine
                .package_intakes(&identity, &cell, after.as_ref())
                .map(|v| Reply::PackageIntakes(Box::new(v))),
            Command::ExportDraftCompileInput {
                identity,
                cell,
                id,
                source_revision,
                binding_revision,
            } => self
                .engine
                .draft_compile_input(&identity, &cell, &id, source_revision, binding_revision)
                .map(|v| Reply::DraftCompileInput(Box::new(v))),
            Command::DraftBindingCatalog { identity, cell } => self
                .engine
                .draft_binding_catalog(&identity, &cell)
                .map(|v| Reply::DraftBindingCatalog(Box::new(v))),
            Command::DraftDeviceBindingCatalog { identity, input } => self
                .engine
                .draft_device_binding_catalog(&identity, &input.cell, input.device_plans)
                .map(|v| Reply::DraftBindingCatalog(Box::new(v))),
            Command::SaveDraftBindings {
                identity,
                key,
                input,
            } => self
                .engine
                .save_draft_bindings(&identity, &key, input)
                .map(|v| Reply::DraftBindingVersion(Box::new(v))),
            Command::GetDraftBindings {
                identity,
                cell,
                id,
                revision,
            } => self
                .engine
                .draft_bindings(&identity, &cell, &id, revision)
                .map(|v| Reply::DraftBindingView(Box::new(v))),
            Command::SaveProcessDraft {
                identity,
                key,
                prepared,
            } => self
                .engine
                .save_process_draft(&identity, &key, prepared)
                .map(|d| Reply::ProcessDraft(Box::new(d))),
            Command::GetProcessDraft {
                identity,
                cell,
                id,
                revision,
            } => self
                .engine
                .process_draft(&identity, &cell, &id, revision)
                .map(|d| Reply::ProcessDraft(Box::new(d))),
            Command::ListProcessDrafts {
                identity,
                cell,
                after,
            } => self
                .engine
                .process_drafts(&identity, &cell, after.as_ref())
                .map(|d| Reply::ProcessDrafts(Box::new(d))),
            Command::ConfigureHostServices(targets) => {
                if targets.len() > 64
                    || targets
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != targets.len()
                {
                    return Err(StoreError::Rejected(
                        rx_domain::fault::Rejection::InvalidInput,
                    ));
                }
                if self.service_health.configured() {
                    return Err(StoreError::Rejected(rx_domain::fault::Rejection::Busy));
                }
                let owners = targets
                    .into_iter()
                    .map(|target| self.engine.service_owner(target))
                    .collect::<rx_ports::Result<Vec<_>>>()?;
                self.service_health.initialize(
                    self.engine.installation.runtime_boot.clone(),
                    owners.clone(),
                )?;
                Ok(Reply::HostServiceOwners(owners))
            }
            Command::ReplaceHostService(old) => {
                let new = self.engine.service_owner(old.target.clone())?;
                self.service_health.replace(&old, new.clone())?;
                Ok(Reply::HostServiceOwner(Box::new(new)))
            }
            Command::PublishHostService(input) => {
                self.service_health
                    .publish(*input, self.engine.current_time())?;
                Ok(Reply::Done)
            }
            Command::IngestHostRead(input) => self
                .engine
                .ingest_host_read(*input)
                .map(Reply::Observations),
            Command::CheckMaintainedConditions => self
                .engine
                .check_maintained_conditions()
                .map(Reply::MaintainedRevoked),
            Command::ReportFact { identity, fact } => self
                .engine
                .report_fact(&identity, fact)
                .map(|()| Reply::Done),
            Command::PrepareHostRenewal(id) => self
                .engine
                .prepare_host_renewal(&id)
                .map(|r| Reply::HostRenewal(Box::new(r))),
            Command::CommitHostRenewal { renewal, grant } => self
                .engine
                .commit_host_renewal(*renewal, grant)
                .map(|r| Reply::HostRegistration(Box::new(r))),
            Command::PrepareHostLink(input) => self
                .engine
                .prepare_host_link(*input)
                .map(|r| Reply::HostLink(Box::new(r))),
            Command::CommitHostLink(input) => self
                .engine
                .commit_host_link(*input)
                .map(|r| Reply::HostRegistration(Box::new(r))),
            Command::CurrentTime => Ok(Reply::Time(self.engine.current_time())),
            Command::BoundHostLink(id) => self
                .engine
                .bound_host_link(&id)
                .map(|r| Reply::HostRegistration(Box::new(r))),
            Command::RequestRuntimeStop => self
                .engine
                .request_runtime_stop()
                .map(|r| Reply::RuntimeStop(Box::new(r))),
            Command::CommitRuntimeProcessStop => self
                .engine
                .commit_runtime_process_stop()
                .map(|r| Reply::RuntimeStop(Box::new(r))),
            Command::RuntimeLifecycle => {
                self.engine.runtime_lifecycle().map(Reply::RuntimeLifecycle)
            }
            Command::PutTerminal {
                identity,
                terminal,
                expected,
            } => self
                .engine
                .put_terminal(&identity, terminal, expected)
                .map(Reply::Revision),
            Command::OpenTerminalUserSession {
                principal,
                id,
                ttl_ns,
                certificate,
            } => self
                .engine
                .authenticated_terminal_user_session(&principal, id, ttl_ns, certificate)
                .map(Reply::Session),
            Command::OpenOperatorPeer {
                principal,
                peer_boot,
                authentication_binding,
            } => self
                .engine
                .open_operator_peer(&principal, peer_boot, authentication_binding)
                .map(Reply::Session),
            Command::NegotiateOperatorCell {
                identity,
                definition,
            } => self
                .engine
                .negotiate_operator_cell(&identity, definition)
                .map(Reply::CellName),
            Command::InspectPeerCell { identity, cell } => self
                .engine
                .inspect_peer_cell(&identity, &cell)
                .map(|(r, c)| Reply::Cell(r, Box::new(c))),
            Command::AdmitClosePolicy {
                identity,
                policy,
                reference,
            } => self
                .engine
                .admit_close_policy(&identity, policy, reference)
                .map(|()| Reply::Done),
            Command::PrepareClose {
                identity,
                key,
                command,
            } => self
                .engine
                .prepare_close(&identity, key.as_str(), command)
                .map(|v| Reply::Clearance(Box::new(v))),
            Command::CloseWithoutRestart {
                identity,
                key,
                command,
            } => self
                .engine
                .close_without_restart(&identity, key.as_str(), command)
                .map(|v| Reply::CloseReceipt(Box::new(v))),
            Command::AdmitProcedure {
                identity,
                policy,
                reference,
            } => self
                .engine
                .admit_procedure(&identity, policy, reference)
                .map(|()| Reply::Done),
            Command::RecordProcedure {
                identity,
                key,
                command,
            } => self
                .engine
                .record_procedure(&identity, key.as_str(), *command)
                .map(|r| Reply::ProcedureReceipt(Box::new(r))),
            Command::OpenCase {
                identity,
                key,
                command,
            } => self
                .engine
                .open_case(&identity, key.as_str(), command)
                .map(|v| Reply::CaseSnapshot(Box::new(v))),
            Command::Cases { identity, cell } => self
                .engine
                .cases(&identity, &cell)
                .map(|v| Reply::Cases(Box::new(v))),
            Command::InspectCase {
                identity,
                cell,
                case,
            } => self
                .engine
                .inspect_case(&identity, &cell, &case)
                .map(|v| Reply::CaseDetail(Box::new(v))),
            Command::AcknowledgeCase {
                identity,
                key,
                command,
            } => self
                .engine
                .acknowledge_case(&identity, key.as_str(), command)
                .map(|v| Reply::CaseDetail(Box::new(v))),
            Command::ProductionView { identity, run } => self
                .engine
                .production_view(&identity, &run)
                .map(|v| Reply::ProductionView(Box::new(v))),
            Command::ExecutorCompletePart {
                identity,
                key,
                command,
            } => self
                .engine
                .executor_complete_part(&identity, key.as_str(), command)
                .map(Reply::PartSnapshot),
            Command::PrepareCheckpoint { identity, target } => self
                .engine
                .prepare_process_checkpoint(&identity, target)
                .map(|v| Reply::CheckpointPreparation(Box::new(v))),
            Command::CommitCheckpoint {
                identity,
                key,
                command,
            } => self
                .engine
                .commit_process_checkpoint(&identity, key.as_str(), *command)
                .map(|v| Reply::RunCheckpoint(Box::new(v))),
            Command::RecordReconciliationObservations {
                identity,
                operation,
                request,
                observations,
            } => self
                .engine
                .record_reconciliation_observations(&identity, &operation, &request, &observations)
                .map(|()| Reply::Done),
            Command::RequestReconciliation {
                identity,
                operation,
            } => self
                .engine
                .request_reconciliation(&identity, &operation)
                .map(|w| Reply::Work(Box::new(w))),
            Command::PendingReconciliations {
                identity,
                after,
                limit,
            } => self
                .engine
                .pending_reconciliations(&identity, after.as_ref(), limit)
                .map(Reply::ReconciliationRequests),
            Command::PlanReconciliation {
                identity,
                operation,
                request,
            } => self
                .engine
                .plan_reconciliation(&identity, &operation, &request)
                .map(|p| Reply::ReconciliationPlan(Box::new(p))),
            Command::UpdateReconciliation {
                identity,
                operation,
                request,
                state,
                issue,
            } => self
                .engine
                .update_reconciliation(&identity, &operation, &request, state, issue)
                .map(|()| Reply::Done),
            Command::PauseExecutorRun {
                identity,
                key,
                command,
            } => self
                .engine
                .pause_executor_run(&identity, key.as_str(), command)
                .map(|r| Reply::RunCheckpoint(Box::new(r))),
            Command::ExecutionSnapshot {
                identity,
                run,
                visit,
            } => self
                .engine
                .execution_snapshot(&identity, &run, visit)
                .map(|v| Reply::ExecutionSnapshot(Box::new(v))),
            Command::ExecutorArtifact {
                identity,
                run,
                reference,
            } => self
                .engine
                .executor_artifact(&identity, &run, &reference)
                .map(Reply::ArtifactBytes),
            Command::ExecutorSubmit {
                identity,
                key,
                command,
            } => self
                .engine
                .executor_submit(&identity, key.as_str(), *command)
                .map(Reply::AdmissionReceipt),
            Command::ExecutorWork {
                identity,
                operation,
            } => self
                .engine
                .executor_work(&identity, &operation)
                .map(|w| Reply::Work(Box::new(w))),
            Command::ExecutorBeginPart {
                identity,
                key,
                command,
            } => self
                .engine
                .executor_begin_part(&identity, key.as_str(), command)
                .map(Reply::PartSnapshot),
            Command::ExecutorResolveActivation {
                identity,
                key,
                command,
            } => self
                .engine
                .executor_resolve_activation(&identity, key.as_str(), command)
                .map(Reply::ActivationSnapshot),
            Command::OpenExecutorPeer {
                principal,
                peer_boot,
                authentication_binding,
            } => self
                .engine
                .open_executor_peer(&principal, peer_boot, authentication_binding)
                .map(Reply::Session),
            Command::InspectServicePeer(identity) => self
                .engine
                .inspect_service_peer(&identity)
                .map(Reply::ServicePeer),
            Command::NegotiateExecutorCell {
                identity,
                definition,
            } => self
                .engine
                .negotiate_executor_cell(&identity, definition)
                .map(Reply::CellName),
            Command::ExecutorRunCheckpoint { identity, run } => self
                .engine
                .executor_run_checkpoint(&identity, &run)
                .map(|s| Reply::RunCheckpoint(Box::new(s))),
            Command::RunCheckpoint { identity, run } => self
                .engine
                .run_checkpoint(&identity, &run)
                .map(|snapshot| Reply::RunCheckpoint(Box::new(snapshot))),
            Command::CheckpointArtifact {
                identity,
                run,
                reference,
            } => self
                .engine
                .checkpoint_artifact(&identity, &run, &reference)
                .map(Reply::ArtifactBytes),
            Command::ProcessProgress {
                identity,
                run,
                visit,
            } => self
                .engine
                .process_progress(&identity, &run, visit)
                .map(|progress| Reply::ProcessProgress(Box::new(progress))),
            Command::ChooseBranch {
                identity,
                key,
                run,
                node,
                visit,
                expected_run,
            } => self
                .engine
                .choose_process_branch(&identity, key.as_str(), &run, &node, visit, expected_run)
                .map(Reply::BranchChoice),
            Command::StartProcessWait {
                identity,
                key,
                run,
                node,
                visit,
                expected_run,
            } => self
                .engine
                .start_process_wait(&identity, key.as_str(), &run, &node, visit, expected_run)
                .map(Reply::WaitWindow),
            Command::CheckProcessWait {
                identity,
                run,
                node,
                visit,
                expected_run,
            } => self
                .engine
                .check_process_wait(&identity, &run, &node, visit, expected_run)
                .map(Reply::WaitProgress),
            Command::PendingDeliveries { after, limit } => self
                .engine
                .pending_deliveries_after(after.as_ref(), limit)
                .map(Reply::PendingDeliveries),
            Command::PlanDelivery { identity, message } => self
                .engine
                .plan_delivery(&identity, &message)
                .map(|p| Reply::DeliveryPlan(Box::new(p))),
            Command::DeliveryAttention { message, issue } => self
                .engine
                .note_delivery_attention(&message, issue)
                .map(|()| Reply::Done),
            Command::RecordReceipt {
                identity,
                message,
                receipt,
            } => self
                .engine
                .record_host_receipt(&identity, &message, receipt)
                .map(|w| Reply::Work(Box::new(w))),
            Command::FinishArm {
                identity,
                message,
                ack,
            } => self
                .engine
                .finish_arm_delivery(&identity, &message, ack)
                .map(Reply::Attempt),
            Command::FinishFence {
                identity,
                message,
                ack,
            } => self
                .engine
                .finish_fence_delivery(&identity, &message, ack)
                .map(|()| Reply::Done),
            Command::ReconciliationWork {
                identity,
                after,
                limit,
            } => self
                .engine
                .reconciliation_work(&identity, after.as_ref(), limit)
                .map(Reply::ReconciliationWork),
            Command::InspectWork {
                identity,
                operation,
            } => self
                .engine
                .inspect_work(&identity, &operation)
                .map(|w| Reply::Work(Box::new(w))),
            Command::InspectRun { identity, run } => self
                .engine
                .inspect_run(&identity, &run)
                .map(|(r, v)| Reply::VersionedRun(r, v)),
            Command::RuntimeRestrictions { identity, cell } => self
                .engine
                .runtime_restrictions(&identity, &cell)
                .map(|v| Reply::RuntimeRestrictions(Box::new(v))),
            Command::BeginPart {
                identity,
                key,
                run,
                expected_budget,
            } => self
                .engine
                .begin_part(&identity, key.as_str(), &run, expected_budget)
                .map(Reply::Part),
            Command::ResolveActivation {
                identity,
                run,
                node,
                visit,
                expected_run,
            } => self
                .engine
                .resolve_activation(&identity, &run, &node, visit, expected_run)
                .map(Reply::Activation),
            Command::SubmitWork {
                identity,
                key,
                command,
            } => self
                .engine
                .submit(&identity, key.as_str(), *command)
                .map(|w| Reply::Work(Box::new(w))),
            Command::ReleaseResources {
                identity,
                key,
                command,
            } => self
                .engine
                .release_resources(&identity, key.as_str(), command)
                .map(|w| Reply::Work(Box::new(w))),
            Command::CompletePart {
                identity,
                key,
                part,
                expected_run,
            } => self
                .engine
                .complete_part(&identity, key.as_str(), &part, expected_run)
                .map(Reply::Part),
            Command::OpenEvidenceProducer {
                principal,
                peer_boot,
                journal,
                authentication_binding,
            } => self
                .engine
                .open_evidence_producer(&principal, peer_boot, journal, authentication_binding)
                .map(Reply::Session),
            Command::NegotiateEvidenceCell {
                identity,
                definition,
            } => self
                .engine
                .negotiate_evidence_cell(&identity, definition)
                .map(Reply::CellName),
            Command::InspectEvidenceProducer(identity) => self
                .engine
                .inspect_evidence_producer(&identity)
                .map(Reply::Producer),
            Command::CurrentEvidenceProducer(principal) => self
                .engine
                .current_evidence_producer(&principal)
                .map(Reply::Producer),
            Command::IngestEvidence { identity, batch } => self
                .engine
                .ingest_evidence_commit(&identity, batch)
                .map(Reply::EvidenceCommit),
            Command::PublishEvidence { identity, batch } => self
                .engine
                .publish_evidence(&identity, batch)
                .map(Reply::EvidenceCommit),
            Command::GetNativeEvidence { identity, evidence } => self
                .engine
                .inspect_native_evidence(&identity, &evidence)
                .map(Reply::NativeEvidence),
            Command::Installation => Ok(Reply::Installation(self.engine.installation.clone())),
            Command::OpenUserSession {
                principal,
                id,
                ttl_ns,
            } => self
                .engine
                .authenticated_user_session(&principal, id, ttl_ns)
                .map(Reply::Session),
            Command::UserProfile(identity) => {
                self.engine.session_profile(&identity).map(Reply::Profile)
            }
            Command::EndUserSession(identity) => self
                .engine
                .end_user_session(&identity)
                .map(|()| Reply::Done),
            Command::Overview(identity) => {
                let mut view = self.engine.overview(&identity)?;
                self.service_health.decorate(&mut view);
                Ok(Reply::Overview(Box::new(view)))
            }
            Command::InspectCell { identity, cell } => self
                .engine
                .inspect_cell(&identity, &cell)
                .map(|(r, v)| Reply::Cell(r, Box::new(v))),
            Command::InstallCell {
                identity,
                key,
                configuration,
            } => self
                .engine
                .install_cell_request(&identity, key.as_str(), *configuration)
                .map(Reply::Revision),
            Command::PutPrincipal {
                identity,
                value,
                expected,
            } => self
                .engine
                .put_principal(&identity, value, expected)
                .map(Reply::Revision),
            Command::CreateRun {
                identity,
                key,
                command,
            } => self
                .engine
                .create_run(&identity, key.as_str(), command)
                .map(Reply::Run),
            Command::StartRun {
                identity,
                key,
                command,
            } => self
                .engine
                .start_run(&identity, key.as_str(), command)
                .map(Reply::Attempt),
            Command::Hold {
                identity,
                key,
                cell,
            } => self
                .engine
                .hold(&identity, key.as_str(), &cell)
                .map(|v| Reply::Held(Box::new(v))),
        }
    }
}

pub type CallResult = Result<Reply, WriterError<StoreError>>;
pub type CallFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = CallResult> + Send + 'a>>;
/// Type-erased service boundary; implemented by the actual dedicated writer handle.
pub trait ApplicationPort: Send + Sync {
    fn request(&self, command: Command) -> CallFuture<'_>;
    fn status(&self) -> crate::writer::Status;
}
impl<P: Processor<Command = Command, Reply = Reply, Error = StoreError>> ApplicationPort
    for Handle<P>
{
    fn request(&self, command: Command) -> CallFuture<'_> {
        Box::pin(self.call(command))
    }
    fn status(&self) -> crate::writer::Status {
        self.status()
    }
}

/// The only service-owned access to the application. No repository handle escapes.
pub struct Handle<P: Processor<Command = Command, Reply = Reply, Error = StoreError>>(Writer<P>);
impl<P: Processor<Command = Command, Reply = Reply, Error = StoreError>> Clone for Handle<P> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<P: Processor<Command = Command, Reply = Reply, Error = StoreError>> Handle<P> {
    pub fn new(writer: Writer<P>) -> Self {
        Self(writer)
    }
    pub async fn call(&self, command: Command) -> Result<Reply, WriterError<StoreError>> {
        self.0.enqueue(command)?.wait().await
    }
    pub fn status(&self) -> crate::writer::Status {
        self.0.status()
    }
    pub fn close(&self) {
        self.0.close()
    }
    pub async fn closed(&self) -> crate::writer::Status {
        self.0.closed().await
    }
}
