//! Exact-configuration requalification evidence. Review is not activation or a physical test.
use crate::{Environment, Identity, process_change::FenceTarget};
use rx_domain::{canonical, types::*};
use rx_package::SignatureEnvelope;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_ARTIFACT: u64 = 8 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 32 * 1024 * 1024;
fn hash(domain: &str, v: &impl Serialize) -> Result<Digest, String> {
    canonical::digest(domain, v).map_err(|e| e.to_string())
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Area {
    Software,
    Equipment,
    CellIntegration,
    Recovery,
    Protection,
    Operations,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: Name,
    pub area: Area,
    pub specification: ArtifactRef,
    pub evidence_schema: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub purposes: BTreeSet<Name>,
    pub cell: Name,
    pub configuration: ArtifactRef,
    pub envelope: ArtifactRef,
    pub definition: ArtifactRef,
    pub environment: Environment,
    pub acceptance_plan: ArtifactRef,
    pub limitations: ArtifactRef,
    pub dependencies: Vec<ArtifactRef>,
    pub criteria: Vec<Criterion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub id: Name,
    pub public_key: Digest,
    pub validators: BTreeSet<Digest>,
    pub environments: BTreeSet<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: Name,
    pub profiles: Vec<Profile>,
    pub keys: Vec<Key>,
}
impl Policy {
    pub fn digest(&self) -> Result<Digest, String> {
        let all = BTreeSet::from([
            Area::Software,
            Area::Equipment,
            Area::CellIntegration,
            Area::Recovery,
            Area::Protection,
            Area::Operations,
        ]);
        if !matches!(
            self.schema.as_str(),
            "rx.requalification-policy.v1" | "rx.requalification-policy.v2"
        ) || self.profiles.is_empty()
            || self.profiles.len() > 64
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
                .profiles
                .iter()
                .map(|p| (&p.cell, p.configuration.sha256))
                .collect::<BTreeSet<_>>()
                .len()
                != self.profiles.len()
        {
            return Err("qualification policy shape".into());
        }
        for k in &self.keys {
            if k.validators.is_empty()
                || k.validators.len() > 128
                || k.environments.is_empty()
                || k.environments
                    .iter()
                    .any(|e| !matches!(e.as_str(), "SIMULATION" | "PHYSICAL"))
            {
                return Err("qualification signer scope".into());
            }
        }
        for p in &self.profiles {
            if (self.schema.as_str() == "rx.requalification-policy.v1" && !p.purposes.is_empty())
                || (self.schema.as_str() == "rx.requalification-policy.v2"
                    && (p.purposes.is_empty()
                        || p.purposes
                            .iter()
                            .any(|v| !matches!(v.as_str(), "PRODUCTION" | "SETUP" | "RECOVERY"))))
            {
                return Err("qualification policy purpose scope differs".into());
            }
            if p.criteria.is_empty()
                || p.criteria.len() > 128
                || p.criteria
                    .iter()
                    .map(|c| &c.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != p.criteria.len()
                || p.criteria.iter().map(|c| c.area).collect::<BTreeSet<_>>() != all
                || p.dependencies.is_empty()
                || p.dependencies.len() > 128
            {
                return Err("all six qualification areas and exact dependencies required".into());
            }
            for a in p.references() {
                if a.size_bytes.0 == 0 || a.size_bytes.0 > MAX_ARTIFACT {
                    return Err("qualification policy artifact bound".into());
                }
            }
        }
        let mut sorted = self.clone();
        sorted.profiles.sort_by(|a, b| {
            (&a.cell, a.configuration.sha256).cmp(&(&b.cell, b.configuration.sha256))
        });
        sorted.keys.sort_by(|a, b| a.id.cmp(&b.id));
        for p in &mut sorted.profiles {
            p.criteria.sort_by(|a, b| a.id.cmp(&b.id));
            p.dependencies
                .sort_by(|a, b| (a.sha256, &a.schema_id).cmp(&(b.sha256, &b.schema_id)));
        }
        hash("RX-REQUALIFICATION-POLICY-v1", &sorted)
    }
}
impl Profile {
    pub fn references(&self) -> Vec<&ArtifactRef> {
        let mut v = vec![
            &self.configuration,
            &self.envelope,
            &self.definition,
            &self.acceptance_plan,
            &self.limitations,
        ];
        v.extend(self.dependencies.iter());
        v.extend(self.criteria.iter().map(|c| &c.specification));
        v
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTarget {
    pub profile: Profile,
    pub expected_revision: Counter,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub blocks: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub id: Id,
    pub change: Id,
    pub change_revision: Counter,
    pub application_digest: Digest,
    pub origin: Name,
    pub runtime_boot: Id,
    pub policy_digest: Digest,
    pub cells: Vec<CellTarget>,
    pub fences: Vec<FenceTarget>,
}
impl Request {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.schema.as_str() != "rx.requalification-request.v1"
            || self.cells.is_empty()
            || self.cells.len() > 64
            || self
                .cells
                .iter()
                .map(|c| &c.profile.cell)
                .collect::<BTreeSet<_>>()
                .len()
                != self.cells.len()
            || self.cells.iter().any(|c| {
                c.expected_revision.0 == 0
                    || c.epoch.0 == 0
                    || c.scopes.is_empty()
                    || c.scopes.values().any(|v| v.0 == 0)
                    || c.blocks.is_empty()
            })
        {
            return Err("qualification request scope".into());
        }
        hash("RX-REQUALIFICATION-REQUEST-v1", self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub request: Request,
    pub requested_by: Name,
    pub terminal: Name,
    pub requested_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Begin {
    pub id: Id,
    pub change: Id,
    pub cell: Name,
    pub expected_change: Counter,
    pub expected_cells: BTreeMap<Name, Counter>,
    pub policy_digest: Digest,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    NotRun,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub cell: Name,
    pub criterion: Name,
    pub verdict: Verdict,
    pub evidence: Vec<ArtifactRef>,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: Name,
    pub request: Request,
    pub validator: Digest,
    pub checks: Vec<Check>,
}
impl Report {
    pub fn digest(&self) -> Result<Digest, String> {
        self.request.digest()?;
        if self.schema.as_str() != "rx.requalification-report.v1" || self.checks.len() > 8192 {
            return Err("qualification report shape".into());
        }
        hash("RX-REQUALIFICATION-REPORT-v1", self)
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>, String> {
        Ok(format!(
            "RX-REQUALIFICATION-REPORT-SIGNATURE-v1\n{key}\n{}",
            self.digest()?
        )
        .into_bytes())
    }
    pub fn references(&self) -> Vec<ArtifactRef> {
        let mut refs = self
            .request
            .cells
            .iter()
            .flat_map(|c| c.profile.references().into_iter().cloned())
            .collect::<Vec<_>>();
        refs.extend(self.checks.iter().flat_map(|c| c.evidence.iter().cloned()));
        refs
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub review: Id,
    pub cell: Name,
    pub expected: Option<Counter>,
    pub directory: rx_package::PackagePath,
    pub report_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub review: Id,
    pub revision: Counter,
    pub report: Report,
    pub signature: SignatureEnvelope,
    pub digest: Digest,
    pub ready_for_review: bool,
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
    pub report_digest: Digest,
    pub expected: Option<Counter>,
    pub choice: Choice,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub review: Id,
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
    pub job: Job,
    pub version: Option<Version>,
    pub decision: Option<Decision>,
    pub context_current: bool,
    pub fences_confirmed: bool,
    pub approval_current: bool,
    pub activation_authorized: bool,
}
pub struct Verified {
    pub(crate) report: Report,
    pub(crate) signature: SignatureEnvelope,
    pub(crate) blobs: BTreeMap<Digest, Vec<u8>>,
    pub(crate) ready: bool,
    pub(crate) policy: Digest,
}
impl Verified {
    pub fn check(
        job: &Job,
        policy: &Policy,
        report: Report,
        signature: SignatureEnvelope,
        blobs: BTreeMap<Digest, Vec<u8>>,
    ) -> Result<Self, String> {
        let p = policy.digest()?;
        if p != job.request.policy_digest || report.request.digest()? != job.request.digest()? {
            return Err("qualification request or policy differs".into());
        }
        let key = policy
            .keys
            .iter()
            .find(|k| k.id == signature.key && k.validators.contains(&report.validator))
            .ok_or("untrusted qualification signer/validator")?;
        for c in &job.request.cells {
            let profile = policy
                .profiles
                .iter()
                .find(|p| p.cell == c.profile.cell && p.configuration == c.profile.configuration)
                .ok_or("qualification profile missing")?;
            if hash("RX-QUALIFICATION-PROFILE-v1", profile)?
                != hash("RX-QUALIFICATION-PROFILE-v1", &c.profile)?
                || !key.environments.contains(
                    &Name::new(match profile.environment {
                        Environment::Simulation => "SIMULATION",
                        Environment::Physical => "PHYSICAL",
                    })
                    .unwrap(),
                )
            {
                return Err("qualification profile or environment differs".into());
            }
        }
        rx_package::verify_detached_message(
            &report.signing_message(&signature.key)?,
            &signature.signature,
            key.public_key.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let expected: BTreeMap<_, _> = job
            .request
            .cells
            .iter()
            .flat_map(|c| {
                c.profile
                    .criteria
                    .iter()
                    .map(move |r| ((&c.profile.cell, &r.id), &r.evidence_schema))
            })
            .collect();
        let mut found = BTreeSet::new();
        let mut ready = true;
        for result in &report.checks {
            let schema = expected
                .get(&(&result.cell, &result.criterion))
                .ok_or("extra qualification check")?;
            if !found.insert((&result.cell, &result.criterion))
                || result.evidence.len() > 32
                || result.note.trim().is_empty()
                || result.note.chars().count() > 4000
            {
                return Err("qualification check shape".into());
            }
            if result.verdict != Verdict::NotRun
                && (result.evidence.is_empty()
                    || result.evidence.iter().any(|a| a.schema_id != **schema))
            {
                return Err("qualified test evidence/schema missing".into());
            }
            ready &= result.verdict == Verdict::Pass;
        }
        if found.len() != expected.len() {
            return Err("required qualification checks missing; use NOT_RUN explicitly".into());
        }
        let refs = report.references();
        let mut expected_blobs = BTreeSet::new();
        let mut total = 0u64;
        for r in &refs {
            let b = blobs
                .get(&r.sha256)
                .ok_or("qualification artifact missing")?;
            if r.size_bytes.0 == 0
                || r.size_bytes.0 > MAX_ARTIFACT
                || r.size_bytes.0 != b.len() as u64
                || rx_package::content_digest(b) != r.sha256
            {
                return Err("qualification artifact integrity/size".into());
            }
            if expected_blobs.insert(r.sha256) {
                total = total
                    .checked_add(r.size_bytes.0)
                    .ok_or("artifact size overflow")?;
            }
        }
        if total > MAX_TOTAL || expected_blobs != blobs.keys().copied().collect() {
            return Err("qualification artifact set/total differs".into());
        }
        Ok(Self {
            report,
            signature,
            blobs,
            ready,
            policy: p,
        })
    }
}
pub struct Ticket {
    pub(crate) job: Job,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Submit,
    pub(crate) issued: TimePoint,
    pub(crate) policy: Digest,
}
impl Ticket {
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn directory(&self) -> &rx_package::PackagePath {
        &self.input.directory
    }
}
pub enum Preflight {
    Recorded(Box<Version>),
    Verify(Box<Ticket>),
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) verified: Verified,
}
impl Prepared {
    pub fn new(ticket: Ticket, verified: Verified) -> Result<Self, String> {
        if verified.policy != ticket.policy
            || verified.report.digest()? != ticket.input.report_digest
            || verified.report.request.digest()? != ticket.job.request.digest()?
        {
            return Err("qualification worker proof differs".into());
        }
        Ok(Self { ticket, verified })
    }
}

pub struct DecisionTicket {
    pub(crate) job: Job,
    pub(crate) version: Version,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Decide,
    pub(crate) issued: TimePoint,
    pub(crate) policy: Digest,
    pub(crate) blobs: BTreeMap<Digest, Vec<u8>>,
}
impl DecisionTicket {
    pub fn verify(self, policy: &Policy) -> Result<PreparedDecision, String> {
        let v = Verified::check(
            &self.job,
            policy,
            self.version.report.clone(),
            self.version.signature.clone(),
            self.blobs.clone(),
        )?;
        if v.policy != self.policy || !v.ready {
            return Err("qualification report is not fully verified PASS".into());
        }
        Ok(PreparedDecision { ticket: self })
    }
}
pub struct PreparedDecision {
    pub(crate) ticket: DecisionTicket,
}
pub enum DecisionPreflight {
    Recorded(Box<Decision>),
    Verify(Box<DecisionTicket>),
}

pub fn required_dependencies(c: &crate::CellConfiguration) -> BTreeSet<Digest> {
    use rx_domain::intent::Body;
    let mut set = BTreeSet::from([
        c.definition.sha256,
        c.envelope.sha256,
        c.recipe.sha256,
        c.site_config_digest,
    ]);
    for s in &c.steps {
        set.insert(s.intent.profile_digest);
        set.insert(s.intent.site_config_digest);
        set.extend(s.intent.calibration_digests.iter().copied());
        match &s.intent.body {
            Body::Trajectory(v) => {
                set.insert(v.trajectory.sha256);
                set.insert(v.tool_digest);
            }
            Body::Program(v) => {
                set.insert(v.program.sha256);
                set.insert(v.parameter_set.sha256);
            }
            Body::Mode(v) => {
                set.insert(v.transition_profile);
            }
            Body::Control(v) => {
                set.insert(v.stream_profile);
            }
            _ => {}
        }
    }
    set
}
