//! Human start-candidate and attempt read models. These values never authorize execution.
use crate::{Commissioning, Environment, Installation, Purpose, Run, StartAttempt, StartRun};
use rx_domain::{fault::Rejection, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    pub cell: Name,
    pub run: Id,
    pub purpose: Purpose,
    pub budget_limit: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartContext {
    pub installation: Installation,
    pub checked_at: TimePoint,
    pub cell: Name,
    pub cell_revision: Counter,
    pub epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    pub configuration_digest: Digest,
    pub environment: Environment,
    pub commissioning: Option<Commissioning>,
    pub envelope: ArtifactRef,
    pub recipe: ArtifactRef,
    pub site_config_digest: Digest,
    pub maximum_budget: Counter,
    pub run_revision: Counter,
    pub run: Run,
    pub request: StartRun,
    /// Candidate evaluation at checked_at only. StartRun repeats the checks before committing.
    pub can_request: bool,
    pub blocking_reason: Option<Rejection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRequest {
    pub cell: Name,
    pub run: Id,
    pub id: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeadlineStatus {
    WithinDeadline,
    Elapsed,
    ClockChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptContext {
    pub installation: Installation,
    pub checked_at: TimePoint,
    pub run_revision: Counter,
    pub run: Run,
    pub attempt: StartAttempt,
    /// Timing is independent of the stored status; elapsed does not rewrite STARTED or ARMING.
    pub deadline_status: DeadlineStatus,
}
