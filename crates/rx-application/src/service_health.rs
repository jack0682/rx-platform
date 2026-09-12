//! Local supervision telemetry. Never an input to qualification, grant or native admission.
use rx_domain::types::*;
use serde::Serialize;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub host: Name,
    pub cell: Name,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub target: Target,
    pub id: Id,
    pub runtime_boot: Id,
    pub definition: Digest,
    pub envelope: Digest,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Connection {
    WaitingForPeer,
    Connecting,
    Bound,
    Attention,
    Stopped,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Observation {
    Waiting,
    Received,
    Unavailable,
    IntegrityAttention,
    Stopped,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Delivery {
    NotStarted,
    Active,
    Stopped,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub connection: Connection,
    pub observation: Observation,
    pub delivery: Delivery,
    pub last_observation_at: Option<TimePoint>,
    pub last_delivery_pass_at: Option<TimePoint>,
    pub delivery_error_history: bool,
}
#[derive(Clone, Debug)]
pub struct Publish {
    pub owner: Owner,
    pub sequence: Counter,
    pub report: Report,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Availability {
    NotConfigured,
    WaitingReport,
    Fresh,
    Stale,
    ContextMismatch,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub schema: &'static str,
    pub availability: Availability,
    pub sampled_at: Option<TimePoint>,
    pub valid_for_ns: Counter,
    pub connection: Option<Connection>,
    pub observation: Option<Observation>,
    pub delivery: Option<Delivery>,
    pub last_observation_at: Option<TimePoint>,
    pub last_delivery_pass_at: Option<TimePoint>,
    pub observation_recent: bool,
    pub delivery_recent: bool,
    pub delivery_error_history: bool,
}
