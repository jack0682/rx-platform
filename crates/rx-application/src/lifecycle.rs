//! Runtime process lifecycle. Never a physical cell/Host shutdown certificate.
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Serving,
    StopRequested,
    StopCommitted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub runtime_boot: Id,
    pub phase: Phase,
    pub stop_id: Option<Id>,
    pub requested_at: Option<TimePoint>,
    pub stopped_at: Option<TimePoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Attention {
    WorkRetained { operation: Id },
    HostFenceUnconfirmed { cell: Name, host: Name },
    OpenCase { case: Id },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StopReport {
    pub lifecycle: Lifecycle,
    pub attention: Vec<Attention>,
    pub attention_count: Counter,
    pub truncated: bool,
    /// P has not stopped a Host/controller or certified physical support/entry conditions.
    pub physical_shutdown_assessed: bool,
}
