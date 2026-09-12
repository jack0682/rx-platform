//! Typed external reports and policy. Neither is a machine safety certificate.
use crate::{
    Role,
    intervention::{CaseSnapshot, CaseType},
};
use rx_domain::{condition::Condition, fault::Rejection, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Action {
    Acknowledge,
    EntryConditionsReported,
    WorkStarted,
    WorkFinished,
    PersonnelAccounted,
    IsolationStateReported,
    ResetObserved,
    HandoverAccepted,
    ConfigurationReported,
}
impl Action {
    pub fn changes_external_state(self) -> bool {
        matches!(
            self,
            Self::WorkStarted
                | Self::WorkFinished
                | Self::IsolationStateReported
                | Self::ResetObserved
                | Self::ConfigurationReported
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Certainty {
    Reported,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub subject: Name,
    pub predicate: Name,
    pub value: TypedValue,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertions {
    pub schema: Name,
    pub case_id: Id,
    pub case_revision: Counter,
    pub procedure_digest: Digest,
    pub actor: Name,
    pub action: Action,
    pub occurred_at: String,
    pub scope_ids: Vec<Name>,
    pub source: Name,
    pub physical_claims: Vec<Claim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Name>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Name>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<TimePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<TimePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Certainty>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: Name,
    pub action: Action,
    pub actors: Vec<Name>,
    pub required_evidence: Vec<Name>,
    pub conditions: Vec<Condition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub external_procedure: ArtifactRef,
    pub dependencies: Vec<ArtifactRef>,
    pub schema: Name,
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    pub case_types: Vec<CaseType>,
    pub entry_conditions: Vec<Condition>,
    pub steps: Vec<Step>,
    pub maximum_report_age_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: Id,
    pub case: Id,
    pub case_revision: Counter,
    pub action: Action,
    pub actor: Name,
    pub scope_ids: Vec<Name>,
    pub occurred_at: String,
    pub evidence_ids: Vec<Id>,
    pub assertions: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub cell: Name,
    pub expected_cell: Option<Counter>,
    pub expected_case: Counter,
    pub record: Record,
    pub assertions: Option<Assertions>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRecord {
    pub record: Record,
    pub recorded_at: TimePoint,
    pub assertions: Assertions,
    pub external_change: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub case: CaseSnapshot,
    pub record: StoredRecord,
    pub transition_error: Option<Rejection>,
    pub facts_recorded: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub active: bool,
    pub finished: bool,
    pub accounted: bool,
    pub handover: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub people: BTreeMap<Name, Participant>,
    pub entry_record: Option<Id>,
    #[serde(default)]
    pub personnel_record: Option<Id>,
    #[serde(default)]
    pub handover_record: Option<Id>,
    #[serde(default)]
    pub entry_context: Vec<crate::intervention::CaseRef>,
    pub policy: Option<ArtifactRef>,
    pub last_external_change: Option<TimePoint>,
    #[serde(default)]
    pub configuration_changed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Admission {
    pub policy: Policy,
    pub reference: ArtifactRef,
    pub admitted_by: Name,
}
pub fn can_report(roles: &std::collections::BTreeSet<Role>) -> bool {
    (roles.contains(&Role::Operator) || roles.contains(&Role::RecoveryLead))
        && !roles
            .iter()
            .any(|r| matches!(r, Role::Host | Role::Executor | Role::OperatorApi))
}
