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
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Mode {
    #[default]
    Replace,
    RevalidateCurrent,
}
impl Mode {
    fn is_replace(&self) -> bool {
        *self == Self::Replace
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    #[serde(default, skip_serializing_if = "Mode::is_replace")]
    pub mode: Mode,
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
    #[serde(default, skip_serializing_if = "Mode::is_replace")]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_binding_plan: Option<rx_process_contract::host_binding_plan::Plan>,
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
    HostBindingChangeRequired,
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
    pub(crate) mode: Mode,
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
    pub(crate) host_binding_plan: Option<rx_process_contract::host_binding_plan::Plan>,
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
        let mut target = crate::process_review::device_configuration(&ticket.job, None)?;
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
        let unchanged = canonical::bytes(&target).map_err(|e| e.to_string())?
            == canonical::bytes(&ticket.job.configuration).map_err(|e| e.to_string())?;
        match ticket.mode {
            Mode::Replace if unchanged => return Err("configuration is unchanged".into()),
            Mode::RevalidateCurrent if !unchanged || ticket.job.configuration.process.is_none() => {
                return Err(
                    "revalidation requires the exact current compiled configuration".into(),
                );
            }
            _ => {}
        }
        let host_binding_plan = host_binding_plan(&ticket, &target)?;
        Ok(Self {
            ticket,
            verified,
            target,
            origins,
            host_binding_plan,
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

fn host_binding_plan(
    ticket: &Ticket,
    target: &CellConfiguration,
) -> Result<Option<rx_process_contract::host_binding_plan::Plan>, String> {
    use rx_process_contract::host_binding_plan::{DevicePackage, HostTarget, Plan};
    let Some(context) = &ticket.job.device_context else {
        return Ok(None);
    };
    let mut hosts: BTreeMap<Name, HostTarget> = BTreeMap::new();
    for step in &target.steps {
        let h = hosts
            .entry(step.host.clone())
            .or_insert_with(|| HostTarget {
                required_intents: vec![],
                required_conditions: Default::default(),
                device_packages: vec![],
                other_affected_cells: Default::default(),
            });
        h.required_intents
            .push(step.intent.normalized().map_err(|e| e.to_string())?);
        h.required_conditions
            .extend(step.condition_ids.iter().cloned());
    }
    for (host, h) in &mut hosts {
        h.required_intents
            .sort_by_cached_key(|v| v.digest().expect("normalized intent"));
        h.required_intents
            .dedup_by(|a, b| a.digest().expect("normalized") == b.digest().expect("normalized"));
        for d in &context.dependencies {
            if ticket.job.request.binding_selections.values().any(|id| {
                d.plan
                    .definition
                    .candidates
                    .get(id)
                    .is_some_and(|c| &c.step.host == host)
            }) {
                h.device_packages.push(DevicePackage {
                    manifest: d.plan.definition.object.manifest,
                    signature: d.plan.definition.object.signature,
                    catalog: d.plan.definition.catalog.clone(),
                });
            }
        }
        h.device_packages.sort_by_key(|p| (p.manifest, p.signature));
        h.device_packages.dedup_by(|a, b| a == b);
        h.other_affected_cells = ticket
            .impact
            .cells
            .iter()
            .filter(|c| c.id != target.id && c.hosts.contains(host))
            .map(|c| c.id.clone())
            .collect();
    }
    let config_ref = |v: &CellConfiguration| -> Result<ArtifactRef, String> {
        let data = canonical::bytes(v).map_err(|e| e.to_string())?;
        Ok(ArtifactRef {
            schema_id: Name::new("rx.cell-configuration.v1").expect("literal"),
            sha256: rx_package::content_digest(&data),
            size_bytes: Counter(data.len() as u64),
        })
    };
    let result = Plan {
        schema: Name::new("rx.host-binding-plan.v1").expect("literal"),
        installation: context.dependencies[0].job.request.installation.clone(),
        cell: target.id.clone(),
        device_context_digest: context.digest()?,
        process_review_digest: ticket.version.review_digest,
        before_configuration: config_ref(&ticket.job.configuration)?,
        after_configuration: config_ref(target)?,
        definition: target.definition.clone(),
        envelope: target.envelope.clone(),
        environment: Name::new(match target.environment {
            crate::Environment::Simulation => "SIMULATION",
            crate::Environment::Physical => "PHYSICAL",
        })
        .expect("literal"),
        scopes: target.scopes.iter().cloned().collect(),
        hosts,
    };
    result.validate()?;
    Ok(Some(result))
}
