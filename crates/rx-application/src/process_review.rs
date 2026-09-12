//! Independently checked, signed software review evidence. Approval never creates run authority.
mod devices;
use crate::{CellConfiguration, Identity, package_intake::Registration};
pub(crate) use devices::configuration as device_configuration;
pub use devices::{DeviceContext, DeviceDependency};
use rx_domain::{canonical, condition::Condition, types::*};
use rx_package::{PackagePath, SignatureEnvelope, store::StoredPackage};
use rx_process_contract::{
    model::{ActionBinding, CompiledBody, ProcessSource, ResolvedProcess},
    package_review::{Issue, Report, Request},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierKey {
    pub id: Name,
    pub public_key: Digest,
    pub validators: BTreeSet<Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub schema: Name,
    pub keys: Vec<VerifierKey>,
}
impl Authority {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.schema.as_str() != "rx.process-verification-authority.v1"
            || self.keys.is_empty()
            || self.keys.len() > 128
            || self
                .keys
                .iter()
                .map(|k| &k.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.keys.len()
            || self
                .keys
                .iter()
                .any(|k| k.validators.is_empty() || k.validators.len() > 64)
        {
            return Err("verification authority shape".into());
        }
        let mut ordered = self.clone();
        ordered.keys.sort_by(|a, b| a.id.cmp(&b.id));
        canonical::digest("RX-PROCESS-VERIFICATION-AUTHORITY-v1", &ordered)
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_plans: Vec<rx_process_contract::compile_input::BindingPlanRef>,
    pub id: Id,
    pub intake: Id,
    pub cell: Name,
    pub configuration_digest: Digest,
    pub policy_generation: Id,
    pub binding_selections: BTreeMap<Name, Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_context: Option<DeviceContext>,
    pub request: Request,
    pub configuration: CellConfiguration,
    pub submitted_by: Name,
    pub requested_by: Name,
    pub created_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub review: Id,
    pub cell: Name,
    pub expected: Option<Counter>,
    pub directory: PackagePath,
    pub report_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub review_digest: Digest,
    pub checker_digest: Digest,
    pub review: Id,
    pub cell: Name,
    pub revision: Counter,
    pub report_digest: Digest,
    pub report: Report,
    pub signature: SignatureEnvelope,
    pub platform_issues: Vec<Issue>,
    pub source: Option<ArtifactRef>,
    pub resolved: Option<ArtifactRef>,
    pub ready_for_software_approval: bool,
    pub recorded_by: Name,
    pub recorded_at: TimePoint,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Choice {
    Approve,
    Reject,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decide {
    pub review: Id,
    pub cell: Name,
    pub report_revision: Counter,
    pub review_digest: Digest,
    pub expected: Option<Counter>,
    pub choice: Choice,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub review_digest: Digest,
    pub review: Id,
    pub cell: Name,
    pub revision: Counter,
    pub report_revision: Counter,
    pub report_digest: Digest,
    pub choice: Choice,
    pub note: String,
    pub decided_by: Name,
    pub decided_at: TimePoint,
    pub scope: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detail {
    pub latest_report_revision: Option<Counter>,
    pub is_latest: bool,
    pub job: Job,
    pub verification: Option<Version>,
    pub decision: Option<Decision>,
    pub source: Option<ProcessSource>,
    pub resolved: Option<ResolvedProcess>,
    pub context_current: bool,
    pub activation_authorized: bool,
    pub approval_matches_current_review: bool,
}
pub struct Ticket {
    pub(crate) job: Job,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Submit,
    pub(crate) boot: Id,
    pub(crate) registration: Registration,
    pub(crate) issued: TimePoint,
}
impl Ticket {
    pub fn directory(&self) -> &PackagePath {
        &self.input.directory
    }
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn registration(&self) -> &Registration {
        &self.registration
    }
}
pub enum Preflight {
    Recorded(Box<Version>),
    Verify(Box<Ticket>),
}
pub struct Validated {
    pub(crate) stored: StoredPackage,
    pub(crate) report: Report,
    pub(crate) signature: SignatureEnvelope,
    pub(crate) source: Option<ProcessSource>,
    pub(crate) resolved: Option<ResolvedProcess>,
    pub(crate) issues: Vec<Issue>,
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) validated: Validated,
}
impl Prepared {
    pub fn new(ticket: Ticket, validated: Validated) -> Result<Self, String> {
        if validated.report.digest()? != ticket.input.report_digest
            || validated.stored.owner() != &ticket.registration.store_owner
        {
            return Err("review ticket verification differs".into());
        }
        Ok(Self { ticket, validated })
    }
}
pub struct DecisionTicket {
    pub(crate) job: Job,
    pub(crate) version: Version,
    pub(crate) source: Option<ProcessSource>,
    pub(crate) resolved: Option<ResolvedProcess>,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Decide,
    pub(crate) boot: Id,
    pub(crate) registration: Option<Registration>,
    pub(crate) issued: TimePoint,
}
impl DecisionTicket {
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn resolved(&self) -> Option<&ResolvedProcess> {
        self.resolved.as_ref()
    }
    pub fn requires_verification(&self) -> bool {
        self.input.choice == Choice::Approve
    }
}
pub enum DecisionPreflight {
    Recorded(Box<Decision>),
    Verify(Box<DecisionTicket>),
}
pub struct PreparedDecision {
    pub(crate) ticket: DecisionTicket,
    pub(crate) validated: Option<Validated>,
}
impl PreparedDecision {
    pub fn approve(ticket: DecisionTicket, validated: Validated) -> Result<Self, String> {
        if ticket.input.choice != Choice::Approve
            || validated.report.digest()? != ticket.version.report_digest
            || !validated.issues.is_empty()
            || !validated.report.issues.is_empty()
            || validated.resolved.is_none()
        {
            return Err("approval evidence is incomplete".into());
        }
        if canonical::bytes(&validated.source).map_err(|e| e.to_string())?
            != canonical::bytes(&ticket.source).map_err(|e| e.to_string())?
            || canonical::bytes(&validated.resolved).map_err(|e| e.to_string())?
                != canonical::bytes(&ticket.resolved).map_err(|e| e.to_string())?
        {
            return Err("reviewed material differs from freshly verified material".into());
        }
        Ok(Self {
            ticket,
            validated: Some(validated),
        })
    }
    pub fn reject(ticket: DecisionTicket) -> Result<Self, String> {
        if ticket.input.choice != Choice::Reject {
            return Err("positive decision requires verification".into());
        }
        Ok(Self {
            ticket,
            validated: None,
        })
    }
}
fn issue(reason: String) -> Issue {
    Issue {
        code: Name::new("PLATFORM_PROCESS_CHECK_FAILED").expect("literal"),
        location: "package/cell".into(),
        detail: reason.chars().take(400).collect(),
    }
}
impl Validated {
    pub fn check(
        job: &Job,
        stored: StoredPackage,
        authority: &Authority,
        report: Report,
        signature: SignatureEnvelope,
        resolved_bytes: Option<&[u8]>,
    ) -> Result<Self, String> {
        Self::check_with_devices(
            job,
            stored,
            authority,
            report,
            signature,
            resolved_bytes,
            vec![],
        )
    }
    pub fn check_with_devices(
        job: &Job,
        stored: StoredPackage,
        authority: &Authority,
        report: Report,
        signature: SignatureEnvelope,
        resolved_bytes: Option<&[u8]>,
        devices: Vec<crate::device_review::Validated>,
    ) -> Result<Self, String> {
        devices::verify_fresh(job, &stored, devices)?;
        report.validate()?;
        if report.request.digest()? != job.request.digest()?
            || authority.digest()? != job.request.verification_authority_digest
            || stored.object().manifest != job.request.package_manifest
            || stored.object().signature != job.request.package_signature
            || stored.policy_fingerprint() != job.request.package_policy_fingerprint
        {
            return Err("review request/package/authority identity differs".into());
        }
        let key = authority
            .keys
            .iter()
            .find(|k| k.id == signature.key && k.validators.contains(&report.validator_digest))
            .ok_or("untrusted report signer or validator")?;
        rx_package::verify_detached_message(
            &report.signing_message(&signature.key)?,
            &signature.signature,
            key.public_key.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let resolved = match (&report.resolved, resolved_bytes) {
            (Some(reference), Some(bytes))
                if rx_package::content_digest(bytes) == reference.sha256
                    && bytes.len() as u64 == reference.size_bytes.0 =>
            {
                let value =
                    canonical::decode_json::<ResolvedProcess>(bytes).map_err(|e| e.to_string())?;
                if canonical::bytes(&value).map_err(|e| e.to_string())? != bytes {
                    return Err("resolved artifact is not canonical".into());
                }
                Some(value)
            }
            (None, None) => None,
            _ => return Err("resolved report artifact differs".into()),
        };
        let mut source = None;
        let mut issues = Vec::new();
        if let Some(process) = &resolved {
            match check_process(job, stored.package(), process) {
                Ok(value) => source = Some(value),
                Err(reason) => issues.push(issue(reason)),
            }
        }
        Ok(Self {
            stored,
            report,
            signature,
            source,
            resolved,
            issues,
        })
    }
}
fn check_process(
    job: &Job,
    package: &rx_package::VerifiedPackage,
    process: &ResolvedProcess,
) -> Result<ProcessSource, String> {
    use rx_process_contract::{compile_input::CompileInput, source_validation, validation};
    validation::validate(process)?;
    if process.package_digest != Some(package.digest()) {
        return Err("resolved package digest differs".into());
    }
    let read = |p: &str| {
        package
            .file(&PackagePath::new(p).expect("literal"))
            .ok_or_else(|| format!("required signed file missing: {p}"))
    };
    let input: CompileInput =
        canonical::decode_json(read("authoring/compile-input.json")?).map_err(|e| e.to_string())?;
    let source = input.validate()?;
    if job.device_context.is_none() && !input.device_sources.is_empty() {
        return Err("device-plan compile input requires device-aware process review and Host binding validation".into());
    }
    rx_process_contract::source_link::verify(&source, process)?;
    if !source_validation::validate(&source).structurally_valid {
        return Err("source structure is invalid".into());
    }
    if !matches!(&package.manifest().entry,rx_package::EntryPoint::Process {source} if source.as_str()=="process/source.json")
    {
        return Err("process package entry differs".into());
    }
    let signed: ProcessSource =
        canonical::decode_json(read("process/source.json")?).map_err(|e| e.to_string())?;
    let bindings: BTreeMap<Name, ActionBinding> =
        canonical::decode_json(read("process/bindings.json")?).map_err(|e| e.to_string())?;
    if canonical::bytes(&source).map_err(|e| e.to_string())?
        != canonical::bytes(&signed).map_err(|e| e.to_string())?
        || canonical::bytes(&bindings).map_err(|e| e.to_string())?
            != canonical::bytes(&input.bindings).map_err(|e| e.to_string())?
    {
        return Err("signed source/bindings disagree with authoring input".into());
    }
    let candidate_configuration = devices::configuration(job, Some(&input))?;
    let cfg = &candidate_configuration;
    let catalog = canonical::digest(
        "RX-DRAFT-BINDING-CATALOG-v1",
        &(
            cfg.id.clone(),
            &cfg.definition,
            &cfg.envelope,
            cfg.site_config_digest,
            &cfg.steps,
        ),
    )
    .map_err(|e| e.to_string())?;
    let catalog = job
        .device_context
        .as_ref()
        .map_or(catalog, |c| c.catalog_digest);
    if input.cell != job.request.cell || input.catalog_digest != catalog {
        return Err("package belongs to a different cell or binding catalog".into());
    }
    let mut normalized_source = source.clone();
    normalized_source.flows.sort_by(|a, b| a.id.cmp(&b.id));
    for flow in &mut normalized_source.flows {
        flow.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    }
    if rx_package::content_digest(&canonical::bytes(&normalized_source).map_err(|e| e.to_string())?)
        != process.source_digest
        || source.process != process.process
        || canonical::bytes(&source.conditions).map_err(|e| e.to_string())?
            != canonical::bytes(&process.conditions).map_err(|e| e.to_string())?
    {
        return Err("resolved source identity or conditions differ".into());
    }
    if job.request.binding_selections.len() != process.bindings.len()
        || input.bindings.len() != process.bindings.len()
    {
        return Err("binding selection coverage differs".into());
    }
    for (id, action) in &process.bindings {
        let selected = job
            .request
            .binding_selections
            .get(id)
            .ok_or("binding selection missing")?;
        let step = cfg
            .steps
            .iter()
            .find(|s| &s.id == selected)
            .ok_or("selected cell step missing")?;
        let input_action = input.bindings.get(id).ok_or("input binding missing")?;
        if action.host != step.host
            || action.host != input_action.host
            || action.intent.digest().map_err(|e| e.to_string())?
                != step.intent.digest().map_err(|e| e.to_string())?
            || action.intent.digest().map_err(|e| e.to_string())?
                != input_action.intent.digest().map_err(|e| e.to_string())?
        {
            return Err("resolved binding differs from signed input or selected cell step".into());
        }
        if !package
            .manifest()
            .permissions
            .contains(&rx_package::Permission::OperationSubmit {
                operation: id.clone(),
            })
        {
            return Err("operation permission not declared".into());
        }
    }
    for node in validation::nodes(process) {
        match &node.body {
            CompiledBody::Branch { condition, .. } | CompiledBody::Wait { condition, .. } => {
                check_condition(&process.conditions[condition], cfg)?
            }
            CompiledBody::Intervention { .. } => {
                return Err(
                    "intervention policy verification is required before software approval".into(),
                );
            }
            _ => {}
        }
    }
    check_predecessors(
        &process.root,
        cfg,
        &job.request.binding_selections,
        BTreeSet::new(),
    )?;
    Ok(source)
}
fn check_condition(condition: &Condition, cfg: &CellConfiguration) -> Result<(), String> {
    match condition {
        Condition::All { children } | Condition::Any { children } => {
            for child in children {
                check_condition(child, cfg)?;
            }
        }
        Condition::Eq {
            fact, schema, unit, ..
        }
        | Condition::Range {
            fact, schema, unit, ..
        }
        | Condition::SetContains {
            fact, schema, unit, ..
        } => {
            if !cfg
                .fact_specs
                .iter()
                .any(|s| &s.id == fact && &s.schema == schema && &s.unit == unit)
            {
                return Err("process condition lacks a matching current FactSpec".into());
            }
        }
    }
    Ok(())
}

pub fn checker_digest() -> Digest {
    rx_package::content_digest(
        concat!(
            "RX-PLATFORM-PROCESS-CHECKER-v1\0",
            include_str!("process_review.rs"),
            include_str!("process_review/devices.rs"),
            include_str!("device_binding.rs"),
            include_str!("../../rx-process-contract/src/source_link.rs"),
            include_str!("../../rx-process-contract/src/validation.rs"),
            include_str!("../../rx-process-contract/src/source_validation.rs"),
            include_str!("../../rx-domain/src/intent.rs"),
            include_str!("../../rx-domain/src/canonical.rs")
        )
        .as_bytes(),
    )
}

fn check_predecessors(
    node: &rx_process_contract::model::CompiledNode,
    cfg: &CellConfiguration,
    selections: &BTreeMap<Name, Name>,
    mut done: BTreeSet<Name>,
) -> Result<BTreeSet<Name>, String> {
    match &node.body {
        CompiledBody::Operation { binding } => {
            let selected = selections.get(binding).ok_or("binding selection missing")?;
            let step = cfg
                .steps
                .iter()
                .find(|s| &s.id == selected)
                .ok_or("cell step missing")?;
            if !step.predecessors.iter().all(|p| done.contains(p)) {
                return Err("process does not establish configured predecessor order".into());
            }
            done.insert(selected.clone());
        }
        CompiledBody::Sequence { children } => {
            for child in children {
                done = check_predecessors(child, cfg, selections, done)?;
            }
        }
        CompiledBody::ParallelAll { children } => {
            let mut joined = done.clone();
            for child in children {
                joined.extend(check_predecessors(child, cfg, selections, done.clone())?);
            }
            done = joined;
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => {
            let t = check_predecessors(when_true, cfg, selections, done.clone())?;
            let f = check_predecessors(when_false, cfg, selections, done)?;
            done = t.intersection(&f).cloned().collect();
        }
        _ => {}
    }
    Ok(done)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: Id,
    pub intake: Id,
    pub requested_by: Name,
    pub created_at: TimePoint,
    pub report_revision: Option<Counter>,
    pub ready_for_software_approval: bool,
    pub decision: Option<Decision>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub cell: Name,
    pub intake: Id,
    pub reviews: Vec<Summary>,
    pub next: Option<Id>,
}
