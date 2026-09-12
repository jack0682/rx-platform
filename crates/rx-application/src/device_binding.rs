//! Reviewed device bindings as non-executable change candidates. No active configuration writes.
pub use crate::process_change::{Impact, ReviewRef};
use crate::{
    CellConfiguration, CompletionRule, Identity, StepBinding, device_review,
    package_intake::Registration,
};
use rx_domain::{canonical, condition::Condition, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub action: Name,
    pub host: Name,
    pub conditions: BTreeMap<Name, Condition>,
    pub completion_postconditions: Vec<Condition>,
    pub handover_max_age_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Propose {
    pub id: Id,
    pub cell: Name,
    pub review: ReviewRef,
    pub bindings: BTreeMap<Name, Selection>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewImpact {
    pub plan: Id,
    pub cell: Name,
    pub expected: Counter,
    pub plan_digest: Digest,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub binding: Name,
    pub code: Name,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub action: Name,
    pub previous_step_digest: Option<Digest>,
    pub step: StepBinding,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub input: Propose,
    pub before: ArtifactRef,
    pub base_configuration_digest: Digest,
    pub object: rx_package::store::ObjectId,
    pub catalog: ArtifactRef,
    pub builder_digest: Digest,
    pub candidates: BTreeMap<Name, Candidate>,
    pub issues: Vec<Issue>,
    pub impact: Impact,
    pub requires_process_review: bool,
    pub requires_host_binding: bool,
    pub requires_operating_envelope_review: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Proposed,
    ImpactReviewed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: Id,
    pub cell: Name,
    pub revision: Counter,
    pub state: State,
    pub plan_digest: Digest,
    pub definition: Definition,
    pub proposed_by: Name,
    pub proposed_at: TimePoint,
    pub impact_review: Option<crate::process_change::ImpactDecision>,
}
impl Plan {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-DEVICE-BINDING-PLAN-v1", &self.definition).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detail {
    pub plan: Plan,
    pub before: CellConfiguration,
    pub context_current: bool,
    pub device_approval_current: bool,
    pub activation_authorized: bool,
    pub configuration_changed: bool,
    pub application_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: Id,
    pub revision: Counter,
    pub state: State,
    pub plan_digest: Digest,
    pub proposed_by: Name,
    pub proposed_at: TimePoint,
    pub affected_cells: Vec<Name>,
    pub issue_count: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub cell: Name,
    pub plans: Vec<Summary>,
    pub next: Option<Id>,
}
pub(crate) enum Action {
    Propose,
    Review(ReviewImpact),
}
pub struct Ticket {
    pub(crate) action: Action,
    pub(crate) input: Propose,
    pub(crate) before: CellConfiguration,
    pub(crate) job: device_review::Job,
    pub(crate) version: device_review::Version,
    pub(crate) decision: device_review::Decision,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) registration: Registration,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
}
impl Ticket {
    pub fn job(&self) -> &device_review::Job {
        &self.job
    }
    pub fn version(&self) -> &device_review::Version {
        &self.version
    }
}
pub enum Preflight {
    Recorded(Box<Plan>),
    Verify(Box<Ticket>),
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) verified: device_review::Validated,
    pub(crate) candidates: BTreeMap<Name, Candidate>,
    pub(crate) issues: Vec<Issue>,
}
impl Prepared {
    pub fn new(ticket: Ticket, verified: device_review::Validated) -> Result<Self, String> {
        if verified.stored.owner() != &ticket.registration.store_owner
            || verified.stored.policy_fingerprint() != ticket.registration.policy_fingerprint
            || verified.report.digest()? != ticket.version.report_digest
            || !verified.report.passed()
        {
            return Err("binding plan requires fresh approved device proof".into());
        }
        let c = crate::device_catalog::extract(verified.stored.package())?
            .ok_or("device catalog absent")?;
        if c.cell != ticket.before.id
            || c.installation != ticket.job.request.installation
            || ticket.input.bindings.is_empty()
            || ticket.input.bindings.len() > 64
            || canonical::bytes(&ticket.input)
                .map_err(|e| e.to_string())?
                .len()
                > 131_072
        {
            return Err("binding proposal scope/size".into());
        }
        let mut candidates = BTreeMap::new();
        let mut issues = vec![];
        for (id, s) in &ticket.input.bindings {
            let intent = c
                .operations
                .get(&s.action)
                .ok_or("selected device action absent")?
                .clone();
            if s.conditions.keys().collect::<BTreeSet<_>>() != c.condition_ids.iter().collect()
                || s.handover_max_age_ns.0 == 0
                || s.completion_postconditions.len() > 32
            {
                return Err("required device conditions or handover policy missing".into());
            }
            let old = ticket.before.steps.iter().find(|v| &v.id == id);
            for condition in s.conditions.values().chain(&s.completion_postconditions) {
                check_condition(condition, &ticket.before, id, &mut issues)?;
            }
            if !ticket.before.hosts.contains(&s.host) {
                issues.push(issue(
                    id,
                    "HOST_REGISTRATION_REQUIRED",
                    "Host is not in the current cell configuration",
                ));
            }
            if intent.site_config_digest != ticket.before.site_config_digest {
                issues.push(issue(
                    id,
                    "SITE_CONFIGURATION_REVIEW_REQUIRED",
                    "Device intent refers to a different site configuration",
                ));
            }
            let completion = if let Some(table) = &c.outcomes {
                CompletionRule::NativeOutcomes {
                    table: table.clone(),
                    postconditions: s.completion_postconditions.clone(),
                }
            } else {
                issues.push(issue(id,"COMPLETION_UNOBSERVABLE","No native outcome table; explicit recovery/completion policy review is required"));
                CompletionRule::Unobservable
            };
            let step = StepBinding {
                id: id.clone(),
                host: s.host.clone(),
                intent,
                predecessors: vec![],
                conditions: s.conditions.values().cloned().collect(),
                completion,
                condition_ids: s.conditions.keys().cloned().collect(),
                condition_revision: old.map_or(Ok(Counter(1)), |v| {
                    v.condition_revision.increment().map_err(|e| e.to_string())
                })?,
                handover_max_age_ns: s.handover_max_age_ns,
            };
            candidates.insert(
                id.clone(),
                Candidate {
                    action: s.action.clone(),
                    previous_step_digest: old
                        .map(|v| {
                            canonical::digest("RX-DRAFT-BINDING-STEP-v1", v)
                                .map_err(|e| e.to_string())
                        })
                        .transpose()?,
                    step,
                },
            );
        }
        if issues.len() > 128 {
            return Err("too many unresolved binding conditions".into());
        }
        Ok(Self {
            ticket,
            verified,
            candidates,
            issues,
        })
    }
}
fn issue(binding: &Name, code: &str, detail: &str) -> Issue {
    Issue {
        binding: binding.clone(),
        code: Name::new(code).expect("fixed issue"),
        detail: detail.into(),
    }
}
fn check_condition(
    c: &Condition,
    before: &CellConfiguration,
    binding: &Name,
    issues: &mut Vec<Issue>,
) -> Result<(), String> {
    let now = TimePoint {
        clock_id: "device-plan/structure".into(),
        ticks_ns: Counter(0),
    };
    let facts = BTreeMap::new();
    let generations = BTreeMap::new();
    c.evaluate(&rx_domain::condition::Context {
        now: &now,
        facts: &facts,
        generations: &generations,
    })
    .map_err(|e| e.to_string())?;
    fn leaves(c: &Condition, before: &CellConfiguration, binding: &Name, issues: &mut Vec<Issue>) {
        match c {
            Condition::All { children } | Condition::Any { children } => {
                for c in children {
                    leaves(c, before, binding, issues);
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
                if !before
                    .fact_specs
                    .iter()
                    .any(|v| &v.id == fact && &v.schema == schema && &v.unit == unit)
                {
                    issues.push(issue(
                        binding,
                        "FACT_SPEC_REVIEW_REQUIRED",
                        &format!("Missing current fact/schema/unit: {fact}/{schema}/{unit}"),
                    ));
                }
            }
        }
    }
    leaves(c, before, binding, issues);
    Ok(())
}
pub fn builder_digest() -> Digest {
    rx_package::content_digest(
        concat!(
            "RX-DEVICE-BINDING-BUILDER-v1\0",
            include_str!("device_binding.rs"),
            include_str!("../../rx-domain/src/condition.rs"),
            include_str!("../../rx-domain/src/intent.rs")
        )
        .as_bytes(),
    )
}
