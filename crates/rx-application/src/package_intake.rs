//! Package intake decisions. No filesystem I/O runs in the authoritative writer.
use crate::Identity;
use rx_domain::types::*;
use rx_package::{
    Manifest, PackagePath,
    store::{ObjectId, StoredPackage},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub generation: Id,
    pub store_owner: Id,
    pub policy_fingerprint: Digest,
    pub policy_file_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub device_review_authority_digest: Option<Digest>,
    pub review_authority_digest: Option<Digest>,
    pub cell: Name,
    pub configuration_digest: Digest,
    pub registration: Option<Registration>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub id: Id,
    pub cell: Name,
    pub title: String,
    pub relative_path: PackagePath,
    pub object: ObjectId,
    pub configuration_digest: Digest,
    pub policy_generation: Id,
}
/// No Deserialize: only the authoritative preflight can create a ticket.
pub struct Ticket {
    pub(crate) input: Submit,
    pub(crate) identity: Identity,
    pub(crate) request_key: Id,
    pub(crate) boot: Id,
    pub(crate) registration: Registration,
    pub(crate) expires_at: TimePoint,
    pub(crate) issued_at: TimePoint,
}
impl Ticket {
    pub fn relative_path(&self) -> &PackagePath {
        &self.input.relative_path
    }
    pub fn object(&self) -> &ObjectId {
        &self.input.object
    }
    pub fn registration(&self) -> &Registration {
        &self.registration
    }
}
pub enum Preflight {
    Recorded(Box<Receipt>),
    Verify(Box<Ticket>),
}
/// Actual immutable bytes from the registered store, never caller-supplied proof fields.
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) stored: StoredPackage,
    pub(crate) device_catalog: Option<rx_process_contract::device_catalog::Catalog>,
}
impl Prepared {
    pub fn new(ticket: Ticket, stored: StoredPackage) -> Result<Self, String> {
        if stored.object() != &ticket.input.object
            || stored.owner() != &ticket.registration.store_owner
            || stored.policy_fingerprint() != ticket.registration.policy_fingerprint
        {
            return Err("intake verification context differs".into());
        }
        let device_catalog = crate::device_catalog::extract(stored.package())?;
        if device_catalog
            .as_ref()
            .is_some_and(|c| c.cell != ticket.input.cell)
        {
            return Err("device catalog belongs to another cell".into());
        }
        Ok(Self {
            ticket,
            stored,
            device_catalog,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_catalog: Option<ArtifactRef>,
    pub id: Id,
    pub cell: Name,
    pub title: String,
    pub object: ObjectId,
    pub configuration_digest: Digest,
    pub registration: Registration,
    pub manifest: Manifest,
    pub submitted_by: Name,
    pub terminal: Option<Name>,
    pub submitted_at: TimePoint,
    pub state: State,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    AwaitingReview,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub receipt: Receipt,
    /// Compares configuration and current service registration only; not fresh content verification.
    pub review_context_current: bool,
    pub content_reverification_required: bool,
    pub activation_authorized: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub cell: Name,
    pub packages: Vec<View>,
    pub next: Option<Id>,
}
