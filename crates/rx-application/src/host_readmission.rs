//! Explicit re-admission of a restarted Host generation. Approval only permits the next
//! host link to replace the recorded registration; it restores no grant, Arm, qualification,
//! permit or Run, and the cell blocks raised by the restart stay latched.
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "rx.host-readmission.v1";

/// ReleaseManager input. Every field names the generation being replaced, never the new one:
/// the new boot is only learned from the authenticated Host link.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approve {
    pub host: Name,
    pub previous_boot: Id,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: Name,
    pub id: Id,
    pub host: Name,
    pub previous_session: Id,
    pub previous_boot: Id,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    /// Registered cells of the replaced generation and the link plan that re-admitted each.
    pub cells: BTreeMap<Name, Option<Id>>,
    pub approved_by: Name,
    pub approved_at: TimePoint,
}
impl Record {
    pub fn complete(&self) -> bool {
        self.cells.values().all(Option::is_some)
    }
}
