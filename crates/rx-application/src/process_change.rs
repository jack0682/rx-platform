//! Approved-process change planning. Staged artifacts are not installed or executable selections.
use crate::{
    CellConfiguration, Identity,
    package_intake::Registration,
    process_review::{Decision, Job, Validated, Version},
};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRef {
    pub id: Id,
    pub revision: Counter,
    pub review_digest: Digest,
    pub decision_revision: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub id: Id,
    pub cell: Name,
    pub review: ReviewRef,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub change: Id,
    pub cell: Name,
    pub expected: Counter,
    pub plan_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewImpact {
    pub target: Transition,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginPreparation {
    pub target: Transition,
    pub refresh: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AffectedCell {
    pub id: Name,
    pub configuration_digest: Digest,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub recipe: ArtifactRef,
    pub hosts: Vec<Name>,
    pub scopes: Vec<Name>,
    pub resources: Vec<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Impact {
    pub cells: Vec<AffectedCell>,
    pub scope_policy: Name,
    pub qualification_review_required: bool,
    pub recovery_review_required: bool,
    pub host_configuration_ack_required: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Proposed,
    ImpactReviewed,
    Staged,
    AppliedUnqualified,
    QualifiedActive,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactDecision {
    pub actor: Name,
    pub note: String,
    pub at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceTarget {
    pub cell: Name,
    pub host: Name,
    pub message: Id,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundCell {
    pub cell: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub change_blocks: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preparation {
    pub attempt: Counter,
    pub runtime_boot: Id,
    pub cells: Vec<BoundCell>,
    pub fences: Vec<FenceTarget>,
    pub started_by: Name,
    pub started_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedCell {
    pub cell: Name,
    pub before: ArtifactRef,
    pub after: ArtifactRef,
    pub previous_qualification: Option<crate::Qualification>,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostProof {
    pub host: Name,
    pub task: Id,
    pub request_digest: Digest,
    pub receipt_digest: Digest,
    pub read_started: TimePoint,
    pub observation_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationRecord {
    pub preparation: Counter,
    pub runtime_boot: Id,
    pub actor: Name,
    pub terminal: Name,
    pub applied_at: TimePoint,
    pub cells: Vec<AppliedCell>,
    pub host_proofs: Vec<HostProof>,
    pub fences: Vec<FenceTarget>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualification_activation: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<ApplicationRecord>,
    pub id: Id,
    pub cell: Name,
    pub revision: Counter,
    pub state: State,
    pub plan_digest: Digest,
    pub builder_digest: Digest,
    pub review: ReviewRef,
    pub before: ArtifactRef,
    pub after: ArtifactRef,
    pub step_origins: BTreeMap<Name, Name>,
    pub impact: Impact,
    pub reason: String,
    pub proposed_by: Name,
    pub proposed_at: TimePoint,
    pub impact_review: Option<ImpactDecision>,
    pub staged_by: Option<Name>,
    pub preparation: Option<Preparation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Blocker {
    PreparationRequired,
    RequalificationRequired,
    ContextChanged,
    ReviewNoLongerApproved,
    PreparationStale,
    RunNeedsDisposition { run: Id },
    OperationUnresolved { operation: Id },
    ResourceRetained { resource: Name, holder: Option<Id> },
    OpenCase { case: Id },
    HostFenceUnconfirmed { cell: Name, host: Name },
    MixedHostConfiguration,
    HostConfigurationOutcomeUnknown,
    HostConfigurationAcknowledgementRequired { cell: Name, host: Name },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detail {
    pub host_configuration: crate::configuration_dispatch::Summary,
    pub change: Change,
    pub before: CellConfiguration,
    pub after: CellConfiguration,
    pub blockers: Vec<Blocker>,
    pub blocker_count: Counter,
    pub blockers_truncated: bool,
    pub applied: bool,
    pub activation_authorized: bool,
}
pub enum Action {
    Propose(Create),
    Stage(Transition),
    Apply(Transition),
}
pub struct Ticket {
    pub(crate) action: Action,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) job: Job,
    pub(crate) version: Version,
    pub(crate) decision: Decision,
    pub(crate) registration: Registration,
    pub(crate) impact: Impact,
    pub(crate) resolved: rx_process_contract::ResolvedProcess,
    pub(crate) issued: TimePoint,
    pub(crate) boot: Id,
}
impl Ticket {
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn resolved(&self) -> &rx_process_contract::ResolvedProcess {
        &self.resolved
    }
}
pub enum Preflight {
    Recorded(Box<Change>),
    Verify(Box<Ticket>),
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) verified: Validated,
    pub(crate) target: CellConfiguration,
    pub(crate) origins: BTreeMap<Name, Name>,
}
impl Prepared {
    pub fn new(ticket: Ticket, verified: Validated) -> Result<Self, String> {
        if !verified.issues.is_empty()
            || !verified.report.issues.is_empty()
            || verified.report.digest()? != ticket.version.report_digest
            || verified.stored.owner() != &ticket.registration.store_owner
            || verified.stored.policy_fingerprint() != ticket.registration.policy_fingerprint
        {
            return Err("change requires current approved package verification".into());
        }
        let resolved = verified
            .resolved
            .as_ref()
            .ok_or("verified process missing")?;
        if canonical::bytes(resolved).map_err(|e| e.to_string())?
            != canonical::bytes(&ticket.resolved).map_err(|e| e.to_string())?
        {
            return Err("reviewed process differs".into());
        }
        let mut target = ticket.job.configuration.clone();
        let mut steps = Vec::new();
        let mut origins = BTreeMap::new();
        for node in rx_process_contract::validation::nodes(resolved) {
            if let rx_process_contract::CompiledBody::Operation { binding } = &node.body {
                let original = ticket
                    .job
                    .request
                    .binding_selections
                    .get(binding)
                    .ok_or("step selection missing")?;
                let mut step = target
                    .steps
                    .iter()
                    .find(|s| &s.id == original)
                    .ok_or("reviewed step missing")?
                    .clone();
                step.id = node.id.clone();
                step.predecessors.clear();
                origins.insert(step.id.clone(), original.clone());
                steps.push(step);
            }
        }
        if steps.is_empty() {
            return Err("installed process must contain at least one device operation".into());
        }
        target.steps = steps;
        let bytes = canonical::bytes(resolved).map_err(|e| e.to_string())?;
        target.recipe = ArtifactRef {
            sha256: rx_process_contract::frontier::resolved_digest(resolved)?,
            schema_id: resolved.schema.clone(),
            size_bytes: Counter(bytes.len() as u64),
        };
        target.process = Some(Box::new(resolved.clone()));
        if canonical::bytes(&target).map_err(|e| e.to_string())?.len() > 1_048_576 {
            return Err("expanded cell configuration exceeds1MiB".into());
        }
        if canonical::bytes(&target).map_err(|e| e.to_string())?
            == canonical::bytes(&ticket.job.configuration).map_err(|e| e.to_string())?
        {
            return Err("configuration is unchanged".into());
        }
        Ok(Self {
            ticket,
            verified,
            target,
            origins,
        })
    }
}
pub fn builder_digest() -> Digest {
    rx_package::content_digest(
        concat!(
            "RX-PROCESS-CHANGE-BUILDER-v1\0",
            include_str!("process_change.rs")
        )
        .as_bytes(),
    )
}
