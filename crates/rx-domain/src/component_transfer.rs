//! Original source cut shared by registration transfer participants; not authority by itself.
use crate::types::*;
use serde::{Deserialize, Serialize};
pub const DECLARATIONS: &str = "components/registration/";
pub const SELECTIONS: &str = "components/resident-selections";
pub const FREEZE: &str = "components/registry-freeze";
pub const FREEZE_SCHEMA: &str = "rx.registration-source-freeze.v1";
pub const REGISTRATION_SCHEMA: &str = "rx.component-registration.v1";
pub const HISTORY_SCHEMA: &str = "rx.component-history.v1";
pub const PREFIXES: [&str; 3] = [DECLARATIONS, SELECTIONS, FREEZE];
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeRequest {
    pub id: Id,
    pub target_installation: Id,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeRecord {
    pub request: FreezeRequest,
    pub declarations_digest: Digest,
    pub declaration_count: Counter,
    pub history_head: Counter,
}

/// Historical P acceptance of the entire frozen cut, viewed through a current scoped peer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAcceptance {
    pub original: FreezeRecord,
    pub receipt_digest: Digest,
    pub accepted_at: crate::types::TimePoint,
    pub component: Id,
    pub scope: Id,
    pub peer: crate::resident_reporting::Peer,
    pub process_ownership: ProcessOwnership,
    pub content_verification: ContentVerification,
    pub work_use_permission: WorkUse,
}
impl TargetAcceptance {
    /// Current delivery peers/scopes can change without rewriting the original decision.
    pub fn same_decision(&self, other: &Self) -> bool {
        self.original == other.original
            && self.receipt_digest == other.receipt_digest
            && self.accepted_at == other.accepted_at
            && self.process_ownership == other.process_ownership
            && self.content_verification == other.content_verification
            && self.work_use_permission == other.work_use_permission
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessOwnership {
    NotTransferred,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContentVerification {
    NotEstablished,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkUse {
    NotEvaluated,
}
