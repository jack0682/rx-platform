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
