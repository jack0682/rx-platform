//! Offline development-release authentication. Not OS authentication or work authority.
//! The only root is a source literal; policy files cannot supply release keys.
use crate::{SignatureEnvelope, content_digest, verify_detached_message};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub mod root;
mod state;
pub use state::{admit, update_revocations};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("release/unsigned")]
    Unsigned,
    #[error("release/invalid-signature")]
    Signature,
    #[error("release/unknown-key")]
    UnknownKey,
    #[error("release/revoked")]
    Revoked,
    #[error("release/rollback")]
    Rollback,
    #[error("release/content-mismatch: {0}")]
    Content(String),
    #[error("release/malformed: {0}")]
    Malformed(String),
    #[error("release/state-invalid: {0}")]
    State(String),
    #[error(transparent)]
    Store(#[from] rx_ports::StoreError),
}
impl Error {
    pub fn condition(&self) -> &'static str {
        match self {
            Self::Unsigned => "release/unsigned",
            Self::Signature => "release/invalid-signature",
            Self::UnknownKey => "release/unknown-key",
            Self::Revoked => "release/revoked",
            Self::Rollback => "release/rollback",
            Self::Content(_) => "release/content-mismatch",
            Self::Malformed(_) => "release/malformed",
            Self::State(_) => "release/state-invalid",
            Self::Store(_) => "release/state-store-failed",
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: Name,
    pub channel: Name,
    pub version: Counter,
    pub inventory_sha256: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRelease {
    pub manifest: Manifest,
    pub signature: Option<SignatureEnvelope>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocations {
    pub schema: Name,
    pub channel: Name,
    pub version: Counter,
    pub revoked: BTreeSet<Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRevocations {
    pub revocations: Revocations,
    pub signature: Option<SignatureEnvelope>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema: String,
    pub files: BTreeMap<String, Digest>,
    pub external_files: BTreeMap<String, Digest>,
}
/// Captured authenticated metadata. Not current admission, immutable disk or permission.
/// Construction authenticates every indexed file through the caller's byte acquisition.
#[derive(Clone, Debug)]
pub struct VerifiedRelease {
    release: SignedRelease,
    revocations: SignedRevocations,
    inventory: Inventory,
    digest: Digest,
}
impl VerifiedRelease {
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn version(&self) -> Counter {
        self.release.manifest.version
    }
}
fn malformed(e: impl std::fmt::Display) -> Error {
    Error::Malformed(e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    canonical::decode_json(bytes).map_err(malformed)
}
/// Domain and signer identity are signed, not inferred from a filename.
pub fn signing_message<T: Serialize>(domain: &str, value: &T, key: &Name) -> Result<Vec<u8>> {
    let mut bytes = domain.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(key.as_str().as_bytes());
    bytes.push(0);
    bytes.extend(canonical::bytes(value).map_err(malformed)?);
    Ok(bytes)
}
fn signature<T: Serialize>(
    domain: &str,
    value: &T,
    envelope: Option<&SignatureEnvelope>,
) -> Result<()> {
    let envelope = envelope.ok_or(Error::Unsigned)?;
    if envelope.key.as_str() != root::KEY_ID {
        return Err(Error::UnknownKey);
    }
    verify_detached_message(
        &signing_message(domain, value, &envelope.key)?,
        &envelope.signature,
        &root::PUBLIC_KEY,
    )
    .map_err(|_| Error::Signature)
}
fn release(value: &SignedRelease) -> Result<Digest> {
    signature("RX-RELEASE-v1", &value.manifest, value.signature.as_ref())?;
    if value.manifest.schema.as_str() != "rx.release.v1"
        || value.manifest.channel.as_str() != root::CHANNEL
        || value.manifest.version.0 == 0
    {
        return Err(malformed("release schema, channel or version"));
    }
    canonical::digest("RX-RELEASE-IDENTITY-v1", &value.manifest).map_err(malformed)
}
fn revocations(value: &SignedRevocations) -> Result<()> {
    signature(
        "RX-RELEASE-REVOCATIONS-v1",
        &value.revocations,
        value.signature.as_ref(),
    )?;
    if value.revocations.schema.as_str() != "rx.release-revocations.v1"
        || value.revocations.channel.as_str() != root::CHANNEL
        || value.revocations.version.0 == 0
        || value.revocations.revoked.len() > 4096
    {
        return Err(malformed("revocation schema, channel, version or size"));
    }
    Ok(())
}
/// Acquire bytes from the declared installation. File names never select a key.
/// Only /usr/bin/python3 is an external runtime dependency in this baseline.
pub fn verify(
    release_bytes: &[u8],
    revocation_bytes: &[u8],
    inventory_bytes: &[u8],
    mut acquire: impl FnMut(&str, bool) -> Result<Vec<u8>>,
) -> Result<VerifiedRelease> {
    if release_bytes.is_empty() || revocation_bytes.is_empty() {
        return Err(Error::Unsigned);
    }
    let signed: SignedRelease = decode(release_bytes)?;
    let revoked: SignedRevocations = decode(revocation_bytes)?;
    let digest = release(&signed)?;
    revocations(&revoked)?;
    if revoked.revocations.revoked.contains(&digest) {
        return Err(Error::Revoked);
    }
    // Sign the exact inventory bytes, preserving detection of even formatting substitution.
    if content_digest(inventory_bytes) != signed.manifest.inventory_sha256 {
        return Err(Error::Content("inventory".into()));
    }
    let inventory: Inventory = decode(inventory_bytes)?;
    if inventory.schema != "rx.solutions-runtime-files.v1"
        || inventory.files.is_empty()
        || inventory.files.len() > 4096
        || inventory.external_files.len() > 1
    {
        return Err(malformed("inventory shape"));
    }
    for (path, expected) in &inventory.files {
        crate::PackagePath::new(path).map_err(malformed)?;
        if !path.is_ascii()
            || matches!(
                path.as_str(),
                "manifests/runtime-files.json"
                    | "manifests/release.json"
                    | "manifests/revocations.json"
            )
        {
            return Err(malformed("reserved or non-ASCII inventory path"));
        }
        if content_digest(&acquire(path, false)?) != *expected {
            return Err(Error::Content(path.clone()));
        }
    }
    for (path, expected) in &inventory.external_files {
        if path != "/usr/bin/python3" {
            return Err(malformed("unsupported external file"));
        }
        if content_digest(&acquire(path, true)?) != *expected {
            return Err(Error::Content(path.clone()));
        }
    }
    Ok(VerifiedRelease {
        release: signed,
        revocations: revoked,
        inventory,
        digest,
    })
}
