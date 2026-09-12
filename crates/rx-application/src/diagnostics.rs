//! Read-only operator diagnostics. These snapshots never authorize a start or a native command.
use crate::model::FactRecord;
use rx_domain::{
    condition::{Condition, Verdict},
    types::*,
};
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct CellDiagnostics {
    pub schema: &'static str,
    /// Conservative presentation lifetime relative to the enclosing Overview.observed_at.
    pub display_valid_for_ns: Counter,
    pub sources: Vec<SourceDiagnostic>,
    pub hosts: Vec<HostDiagnostic>,
    pub conditions: Vec<ConditionDiagnostic>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceDiagnostic {
    pub source: Name,
    pub host: Name,
    pub expected_generation: Option<Id>,
    pub maximum_age_ns: Counter,
    pub maximum_uncertainty_ns: Counter,
    pub age_ns: Option<Counter>,
    pub usable: bool,
    pub issues: Vec<SourceIssue>,
    pub observation: Option<FactRecord>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceIssue {
    MissingObservation,
    HostUnregistered,
    GenerationUnregistered,
    GenerationMismatch,
    WrongSourceHost,
    SchemaMismatch,
    UnitMismatch,
    BadQuality,
    OriginAgeUnbounded,
    Disputed,
    ClockMismatch,
    FutureTimestamp,
    UncertaintyExceeded,
    AgeExceeded,
    AgeOverflow,
}
#[derive(Clone, Debug, Serialize)]
pub struct HostDiagnostic {
    pub runtime: Option<crate::service_health::Snapshot>,
    pub host: Name,
    /// This is the stored authorization context, not a TCP connection or device readiness assertion.
    pub context: HostContext,
    pub grant_valid_until: Option<TimePoint>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostContext {
    Unregistered,
    IdentityUnavailable,
    ContextStale,
    ClockMismatch,
    LeaseExpired,
    Current,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConditionDiagnostic {
    /// Presentation path in this definition, not a frozen ConditionEvaluation identity.
    pub path: String,
    pub group: ConditionGroup,
    pub step: Option<Name>,
    pub expression: Condition,
    pub verdict: Verdict,
    pub reason: ConditionReason,
    pub evidence_ids: Vec<Id>,
    pub valid_until: Option<TimePoint>,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConditionGroup {
    Start,
    Maintained,
    Operation,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConditionReason {
    Satisfied,
    NotSatisfied,
    SourceUnavailable,
    ExpressionMismatch,
}
