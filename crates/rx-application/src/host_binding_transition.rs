//! P-side comparison of a pre-recorded replacement intent with authenticated Host reads.
//! The caller must supply current registered-Host context; this module grants no permission.
use rx_domain::{
    host_configuration::{Observation, Snapshot},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellIdentity {
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub request: Id,
    pub change: Id,
    pub host: Name,
    pub cell: Name,
    pub host_plan_digest: Digest,
    pub before_configuration: Digest,
    pub after_configuration: Digest,
    pub before_cells: BTreeMap<Name, CellIdentity>,
    pub after_cells: BTreeMap<Name, CellIdentity>,
    pub runtime_boot: Id,
    pub created_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub snapshot: Snapshot,
    pub observed_at: TimePoint,
    pub producer_session: Id,
}
/// Internal read context from the P writer/registered transport, never an HTTP proof body.
pub struct ReadContext {
    pub runtime_boot: Id,
    pub producer_session: Id,
    pub host_boot: Id,
    pub delivery_journal: Id,
    pub started_at: TimePoint,
    pub now: TimePoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rejection {
    TransportUnavailable,
    AuthorizationChanged,
    InvalidIntent,
    InvalidObservation,
    StaleRead,
    RuntimeChanged,
    HostGenerationChanged,
    CohortChanged,
    MissingIdentity,
    AlreadyCommittedBeforeBaseline,
    MissingCommit,
    RequestChanged,
    PlanChanged,
    ConfigurationChanged,
    InstallationChanged,
    JournalChanged,
    HostNotRestarted,
}
fn intent_valid(intent: &Intent) -> Result<(), Rejection> {
    if intent.before_cells.is_empty()
        || intent.before_cells.len() > 64
        || !intent.before_cells.contains_key(&intent.cell)
        || intent.before_cells.keys().ne(intent.after_cells.keys())
    {
        return Err(Rejection::InvalidIntent);
    }
    Ok(())
}
fn observed<'a>(
    intent: &Intent,
    value: &'a Observation,
    read: &ReadContext,
) -> Result<&'a Snapshot, Rejection> {
    intent_valid(intent)?;
    value
        .validate()
        .map_err(|_| Rejection::InvalidObservation)?;
    if intent.runtime_boot != read.runtime_boot {
        return Err(Rejection::RuntimeChanged);
    }
    if read
        .now
        .age_ns(&read.started_at)
        .is_none_or(|age| age > 3_000_000_000)
        || read.started_at.age_ns(&intent.created_at).is_none()
    {
        return Err(Rejection::StaleRead);
    }
    let s = &value.snapshot;
    if s.host != intent.host
        || s.host_boot != read.host_boot
        || s.delivery_journal != read.delivery_journal
    {
        return Err(Rejection::HostGenerationChanged);
    }
    if s.installation_identity.is_none() || s.evidence_journal.is_none() {
        return Err(Rejection::MissingIdentity);
    }
    Ok(s)
}
fn cohort(snapshot: &Snapshot, expected: &BTreeMap<Name, CellIdentity>) -> Result<(), Rejection> {
    if snapshot.cells.len() != expected.len()
        || snapshot.cells.iter().any(|c| {
            expected.get(&c.cell).is_none_or(|e| {
                e.definition != c.definition
                    || e.envelope != c.envelope
                    || e.environment != c.environment
            })
        })
    {
        return Err(Rejection::CohortChanged);
    }
    Ok(())
}
pub fn capture_baseline(
    intent: &Intent,
    value: &Observation,
    read: &ReadContext,
) -> Result<Baseline, Rejection> {
    let s = observed(intent, value, read)?;
    cohort(s, &intent.before_cells)?;
    if s.binding_commit
        .as_ref()
        .is_some_and(|c| c.request == intent.request)
    {
        return Err(Rejection::AlreadyCommittedBeforeBaseline);
    }
    Ok(Baseline {
        snapshot: s.clone(),
        observed_at: read.now.clone(),
        producer_session: read.producer_session.clone(),
    })
}
/// A match establishes metadata correlation only. P must separately check current authorization,
/// staged change, fences, resources, process application and qualification before progressing.
pub fn confirm(
    intent: &Intent,
    baseline: &Baseline,
    value: &Observation,
    read: &ReadContext,
) -> Result<(), Rejection> {
    let s = observed(intent, value, read)?;
    cohort(s, &intent.after_cells)?;
    if baseline.snapshot.host != intent.host
        || baseline.snapshot.installation_identity.is_none()
        || baseline.snapshot.evidence_journal.is_none()
    {
        return Err(Rejection::MissingIdentity);
    }
    cohort(&baseline.snapshot, &intent.before_cells)?;
    if read.started_at.age_ns(&baseline.observed_at).is_none() {
        return Err(Rejection::StaleRead);
    }
    if s.host_boot == baseline.snapshot.host_boot {
        return Err(Rejection::HostNotRestarted);
    }
    let c = s.binding_commit.as_ref().ok_or(Rejection::MissingCommit)?;
    if c.request != intent.request {
        return Err(Rejection::RequestChanged);
    }
    if c.plan_digest != intent.host_plan_digest || c.cell != intent.cell {
        return Err(Rejection::PlanChanged);
    }
    if c.before_configuration != intent.before_configuration
        || c.after_configuration != intent.after_configuration
    {
        return Err(Rejection::ConfigurationChanged);
    }
    if Some(c.before_installation_identity) != baseline.snapshot.installation_identity
        || Some(c.after_installation_identity) != s.installation_identity
    {
        return Err(Rejection::InstallationChanged);
    }
    if c.delivery_journal != baseline.snapshot.delivery_journal
        || Some(&c.evidence_journal) != baseline.snapshot.evidence_journal.as_ref()
    {
        return Err(Rejection::JournalChanged);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    AwaitingBaseline,
    BaselineRecorded,
    MetadataMatched,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub intent: Intent,
    pub phase: Phase,
    pub baseline: Option<Baseline>,
    pub observation: Option<Observation>,
    pub read_started: Option<TimePoint>,
    pub observed_session: Option<Id>,
    pub issue: Option<Rejection>,
    pub created_by: Name,
    pub activation_authorized: bool,
}
