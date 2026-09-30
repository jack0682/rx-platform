//! Platform-owned component declarations. Registration does not validate a package or grant work.
use rx_domain::{component::*, types::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub declaration: Declaration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub id: Id,
    pub expected_revision: Counter,
    pub declaration: Declaration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retire {
    pub id: Id,
    pub expected_revision: Counter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verification {
    NotEstablished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Ownership {
    NotEstablishedByRegistration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkUse {
    NotEvaluated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub installation: Id,
    pub registration: Registration,
    pub owner: Name,
    pub created_at: TimePoint,
    pub changed_at: TimePoint,
    pub changed_by: Name,
}

/// A declaration snapshot, including on idempotent response recovery. No execution permission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub revision: Counter,
    pub record: Record,
    pub content_verification: Verification,
    pub execution_ownership: Ownership,
    pub work_use_permission: WorkUse,
}

impl View {
    pub(crate) fn new(revision: Counter, record: Record) -> Self {
        Self {
            revision,
            record,
            content_verification: Verification::NotEstablished,
            execution_ownership: Ownership::NotEstablishedByRegistration,
            work_use_permission: WorkUse::NotEvaluated,
        }
    }
}
