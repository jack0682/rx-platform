//! Shared component identity, independent of process management and operating authority.
//!
//! These values retain the supervisor's v1 serialized representation. A registration is
//! accepted author content, not current process ownership, readiness or permission to work.
//! Its authoritative writer and any migration are application responsibilities.
use crate::types::{Counter, Digest, Id, Name};
use serde::{Deserialize, Serialize};

/// A reference to accepted author content, never an editable policy snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogReference {
    pub program: Name,
    pub digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub label: Name,
    pub catalog: CatalogReference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistrationState {
    Accepted,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub id: Id,
    pub declaration: Declaration,
    pub state: RegistrationState,
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionedRegistration {
    pub revision: Counter,
    pub registration: Registration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub registration: Id,
    pub registration_revision: Counter,
    pub catalog: CatalogReference,
    /// Opaque consumer run and selection identities, not part of the registration key.
    pub run: Id,
    pub selection: Name,
    pub instance: Id,
}
