//! Record of an offline store restore.
//!
//! Restoring a backup replaces the ledger with an older cut. Nothing in that cut proves that
//! Hosts, native work, materials or physical state still match it, so a restore rotates the
//! installation's store generation: every peer pinned to the previous generation is refused at
//! session open until an operator re-pins it and the Host is re-admitted explicitly. Runtime
//! invalidation provenance recorded under the previous generation is no longer readable as
//! current provenance. The restore itself grants nothing.
use rx_domain::types::*;
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "rx.store-restore.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreRestore {
    pub schema: Name,
    pub id: Id,
    pub installation: Id,
    pub previous_generation: Id,
    pub generation: Id,
    /// Runtime boot recorded in the restored cut; the next runtime start replaces it.
    pub previous_runtime_boot: Id,
    /// SHA-256 of the backup file bytes as restored.
    pub backup_sha256: Digest,
    pub restored_at: TimePoint,
}

pub use crate::engine::store_restore::{last_store_restore, restore_store};
