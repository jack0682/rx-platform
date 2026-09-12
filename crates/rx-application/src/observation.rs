//! Internal observation ingestion results; no execution authority is carried by a read.
use rx_domain::{host_snapshot::HostSnapshot, types::*};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug)]
pub struct HostRead {
    pub plan: Id,
    pub snapshot: HostSnapshot,
    pub read_started: TimePoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disposition {
    Current,
    Duplicate,
    Historical,
    GenerationChanged,
    IntegrityConflict,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub source: Name,
    pub evidence: Id,
    pub disposition: Disposition,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchReceipt {
    pub cell: Name,
    pub received_at: TimePoint,
    pub entries: Vec<Entry>,
    pub maintained_revoked: Vec<Name>,
}
impl BatchReceipt {
    pub fn continuity_lost(&self) -> bool {
        self.entries.iter().any(|e| {
            matches!(
                e.disposition,
                Disposition::GenerationChanged | Disposition::IntegrityConflict
            )
        })
    }
}
