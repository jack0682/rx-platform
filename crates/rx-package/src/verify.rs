use crate::model::*;
use ed25519_dalek::{Signature, VerifyingKey};
use rx_domain::{canonical, types::*};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("package input: {0}")]
    Invalid(String),
    #[error("untrusted publisher or package permission")]
    Untrusted,
    #[error("package signature is invalid")]
    Signature,
    #[error("incompatible package target/contract")]
    Incompatible,
    #[error("package file inventory or digest differs: {0}")]
    Content(String),
    #[error("unresolved locked dependency: {0}")]
    Dependency(String),
    #[error("required artifact is unavailable: {0}")]
    Asset(String),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone)]
pub struct VerificationPolicy {
    pub additional_package_abis: BTreeSet<Name>,
    pub publishers: BTreeMap<Name, TrustedPublisher>,
    pub contracts: ContractSet,
    pub target: Target,
    pub dependencies: BTreeMap<Name, Arc<VerifiedPackage>>,
    pub assets: BTreeMap<Digest, ArtifactRef>,
    pub max_files: usize,
    pub max_content_bytes: u64,
}
impl VerificationPolicy {
    fn validate_abis(&self) -> Result<()> {
        if self.additional_package_abis.len() > 8
            || self
                .additional_package_abis
                .contains(&self.contracts.package_abi)
        {
            return Err(Error::Invalid("additional package ABI policy".into()));
        }
        Ok(())
    }
    /// Meaning of the in-memory policy used for a verification, independent of source paths.
    pub fn fingerprint(&self) -> Result<Digest> {
        self.validate_abis()?;
        let publishers = self.publishers.iter().map(|(id,p)| serde_json::json!({
            "id":id,"publisher":p.publisher,"key":Digest::from_bytes(p.verifying_key),"kinds":p.kinds,"permissions":p.permissions
        })).collect::<Vec<_>>();
        let dependencies = self
            .dependencies
            .iter()
            .map(|(id, p)| {
                serde_json::json!({
                    "id":id,"manifest":p.digest(),"signature":p.signature()
                })
            })
            .collect::<Vec<_>>();
        let mut value = serde_json::json!({
            "publishers":publishers,"dependencies":dependencies,"assets":self.assets,
            "contracts":self.contracts,"target":self.target,"max_files":Counter(self.max_files as u64),"max_content_bytes":Counter(self.max_content_bytes)
        });
        if !self.additional_package_abis.is_empty() {
            value["additional_package_abis"] = serde_json::to_value(&self.additional_package_abis)
                .map_err(|e| Error::Invalid(e.to_string()))?;
        }
        canonical::digest("RX-PACKAGE-VERIFICATION-POLICY-v1", &value)
            .map_err(|e| Error::Invalid(e.to_string()))
    }
}
pub struct VerifiedPackage {
    manifest: Manifest,
    digest: Digest,
    files: BTreeMap<PackagePath, Arc<[u8]>>,
    signer: Name,
    verifying_key: [u8; 32],
    signature: SignatureEnvelope,
}
impl VerifiedPackage {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn signature(&self) -> &SignatureEnvelope {
        &self.signature
    }
    pub fn files(&self) -> impl Iterator<Item = (&PackagePath, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path, bytes.as_ref()))
    }
    pub fn file(&self, path: &PackagePath) -> Option<&[u8]> {
        self.files.get(path).map(AsRef::as_ref)
    }
}
pub fn manifest_bytes(manifest: &Manifest) -> Result<Vec<u8>> {
    let mut value = manifest.clone();
    value.files.sort_by(|a, b| a.path.cmp(&b.path));
    value.targets.sort();
    value.permissions.sort();
    value.dependencies.sort_by(|a, b| a.package.cmp(&b.package));
    value.assets.sort_by_key(|a| a.sha256);
    if let EntryPoint::Device { profiles, .. } | EntryPoint::DeviceReference { profiles, .. } =
        &mut value.entry
    {
        profiles.sort();
    }
    canonical::bytes(&value).map_err(|e| Error::Invalid(e.to_string()))
}
pub fn content_digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}
pub fn signing_message(manifest: &Manifest, key: &Name) -> Result<Vec<u8>> {
    let mut value = b"RX-PACKAGE-MANIFEST-v1\0".to_vec();
    value.extend(canonical::bytes(key).map_err(|e| Error::Invalid(e.to_string()))?);
    value.push(0);
    value.extend(manifest_bytes(manifest)?);
    Ok(value)
}
pub fn verify_package(
    manifest_json: &[u8],
    signature_json: &[u8],
    files: BTreeMap<PackagePath, Vec<u8>>,
    policy: &VerificationPolicy,
) -> Result<VerifiedPackage> {
    let manifest: Manifest =
        canonical::decode_json(manifest_json).map_err(|e| Error::Invalid(e.to_string()))?;
    let signature: SignatureEnvelope =
        canonical::decode_json(signature_json).map_err(|e| Error::Invalid(e.to_string()))?;
    validate_manifest(&manifest, policy)?;
    let trusted = policy
        .publishers
        .get(&signature.key)
        .ok_or(Error::Untrusted)?;
    if trusted.publisher != manifest.publisher
        || !trusted.kinds.contains(&manifest.entry.kind())
        || manifest
            .permissions
            .iter()
            .any(|permission| !trusted.permissions.contains(permission))
    {
        return Err(Error::Untrusted);
    }
    let signature_bytes: [u8; 64] = hex(&signature.signature)?
        .try_into()
        .map_err(|_| Error::Signature)?;
    let key = VerifyingKey::from_bytes(&trusted.verifying_key).map_err(|_| Error::Signature)?;
    key.verify_strict(
        &signing_message(&manifest, &signature.key)?,
        &Signature::from_bytes(&signature_bytes),
    )
    .map_err(|_| Error::Signature)?;
    if files.len() != manifest.files.len() {
        return Err(Error::Content("file set".into()));
    }
    let mut total = 0u64;
    for entry in &manifest.files {
        let bytes = files
            .get(&entry.path)
            .ok_or_else(|| Error::Content(entry.path.as_str().into()))?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| Error::Invalid("content size overflow".into()))?;
        if total > policy.max_content_bytes
            || bytes.len() as u64 != entry.size_bytes.0
            || content_digest(bytes) != entry.sha256
        {
            return Err(Error::Content(entry.path.as_str().into()));
        }
    }
    let digest = content_digest(&manifest_bytes(&manifest)?);
    let mut visited = BTreeMap::from([(manifest.package.clone(), digest)]);
    let mut active = BTreeSet::from([manifest.package.clone()]);
    verify_dependencies(&manifest, policy, &mut visited, &mut active)?;
    for asset in &manifest.assets {
        if policy.assets.get(&asset.sha256) != Some(asset) {
            return Err(Error::Asset(asset.sha256.to_string()));
        }
    }
    Ok(VerifiedPackage {
        manifest,
        digest,
        files: files
            .into_iter()
            .map(|(path, bytes)| (path, Arc::from(bytes)))
            .collect(),
        signer: signature.key.clone(),
        signature,
        verifying_key: trusted.verifying_key,
    })
}
fn verify_dependencies(
    manifest: &Manifest,
    policy: &VerificationPolicy,
    visited: &mut BTreeMap<Name, Digest>,
    active: &mut BTreeSet<Name>,
) -> Result<()> {
    if active.len() > 32 || visited.len() > 1024 {
        return Err(Error::Dependency("dependency graph limit".into()));
    }
    for required in &manifest.dependencies {
        if active.contains(&required.package) {
            return Err(Error::Dependency("cycle or root version collision".into()));
        }
        if visited
            .get(&required.package)
            .is_some_and(|digest| *digest != required.manifest_digest)
        {
            return Err(Error::Dependency(
                "conflicting transitive package versions".into(),
            ));
        }
        let package = policy
            .dependencies
            .get(&required.package)
            .ok_or_else(|| Error::Dependency(required.package.to_string()))?;
        if package.manifest.package != required.package
            || package.manifest.version != required.version
            || package.manifest.entry.kind() != required.kind
            || package.digest != required.manifest_digest
        {
            return Err(Error::Dependency(required.package.to_string()));
        }
        let trusted = policy
            .publishers
            .get(&package.signer)
            .ok_or(Error::Untrusted)?;
        if trusted.verifying_key != package.verifying_key
            || trusted.publisher != package.manifest.publisher
            || !trusted.kinds.contains(&package.manifest.entry.kind())
            || package
                .manifest
                .permissions
                .iter()
                .any(|p| !trusted.permissions.contains(p))
        {
            return Err(Error::Untrusted);
        }
        validate_manifest(&package.manifest, policy)?;
        if visited
            .insert(required.package.clone(), required.manifest_digest)
            .is_none()
        {
            active.insert(required.package.clone());
            verify_dependencies(&package.manifest, policy, visited, active)?;
            active.remove(&required.package);
        }
        for asset in &package.manifest.assets {
            if policy.assets.get(&asset.sha256) != Some(asset) {
                return Err(Error::Asset(asset.sha256.to_string()));
            }
        }
    }
    Ok(())
}
/// Structural/target checks only; does not verify a signature, files, dependencies or trust.
pub fn validate_manifest(manifest: &Manifest, policy: &VerificationPolicy) -> Result<()> {
    policy.validate_abis()?;
    let valid_version = match (
        &manifest.entry,
        manifest.schema.as_str(),
        manifest.contracts.package_abi.as_str(),
    ) {
        (EntryPoint::DeviceReference { .. }, "rx.package.v2", "rx.package-abi.v2") => true,
        (EntryPoint::DeviceReference { .. }, _, _) => false,
        (_, "rx.package.v1", _) => true,
        _ => false,
    };
    if !valid_version || !manifest.version.build.is_empty() {
        return Err(Error::Invalid("manifest schema or version".into()));
    }
    if manifest.contracts.base != policy.contracts.base
        || manifest.contracts.cell != policy.contracts.cell
        || (manifest.contracts.package_abi != policy.contracts.package_abi
            && !policy
                .additional_package_abis
                .contains(&manifest.contracts.package_abi))
        || !manifest
            .targets
            .iter()
            .any(|target| target.supports(&policy.target))
    {
        return Err(Error::Incompatible);
    }
    if manifest.targets.is_empty()
        || manifest.targets.iter().collect::<BTreeSet<_>>().len() != manifest.targets.len()
        || manifest.files.is_empty()
        || manifest.files.len() > policy.max_files
        || policy.max_files > 4096
        || manifest.permissions.iter().collect::<BTreeSet<_>>().len() != manifest.permissions.len()
    {
        return Err(Error::Invalid(
            "empty/duplicate or excessive manifest collection".into(),
        ));
    }
    if manifest
        .permissions
        .iter()
        .any(|p| !p.allowed_for(manifest.entry.kind()))
    {
        return Err(Error::Untrusted);
    }
    let mut paths = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.files {
        if !paths.insert(entry.path.as_str().to_ascii_lowercase()) {
            return Err(Error::Invalid("case-colliding or duplicate path".into()));
        }
        if matches!(
            entry.path.as_str().to_ascii_lowercase().as_str(),
            "manifest.json" | "manifest.sig.json"
        ) {
            return Err(Error::Invalid("reserved manifest filename".into()));
        }
        total = total
            .checked_add(entry.size_bytes.0)
            .ok_or_else(|| Error::Invalid("content size overflow".into()))?;
        if total > policy.max_content_bytes {
            return Err(Error::Invalid("content size limit".into()));
        }
    }
    let mut spelling = BTreeMap::new();
    for entry in &manifest.files {
        let mut prefix = String::new();
        for component in entry.path.as_str().split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let folded = prefix.to_ascii_lowercase();
            if spelling.get(&folded).is_some_and(|old| old != &prefix) {
                return Err(Error::Invalid("case-aliased path hierarchy".into()));
            }
            spelling.insert(folded, prefix.clone());
        }
    }
    for path in &paths {
        let mut prefix = String::new();
        let components: Vec<_> = path.split('/').collect();
        for component in &components[..components.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if paths.contains(&prefix) {
                return Err(Error::Invalid("file/directory path collision".into()));
            }
        }
    }
    let entry_paths = manifest.entry.paths();
    if entry_paths.iter().collect::<BTreeSet<_>>().len() != entry_paths.len() {
        return Err(Error::Invalid("entrypoint role collision".into()));
    }
    for path in manifest.entry.paths() {
        if !manifest.files.iter().any(|file| &file.path == path) {
            return Err(Error::Invalid("entry point absent from inventory".into()));
        }
    }
    match &manifest.entry {
        EntryPoint::Device {
            profiles, adapter, ..
        } => {
            if profiles.is_empty()
                || profiles.iter().collect::<BTreeSet<_>>().len() != profiles.len()
                || !manifest
                    .files
                    .iter()
                    .any(|f| &f.path == adapter && f.executable)
            {
                return Err(Error::Invalid("device profile/adapter entry".into()));
            }
        }
        EntryPoint::DeviceReference { profiles, .. } => {
            if profiles.is_empty()
                || profiles.iter().collect::<BTreeSet<_>>().len() != profiles.len()
            {
                return Err(Error::Invalid("device reference profiles".into()));
            }
            if manifest.files.iter().any(|f| f.executable) {
                return Err(Error::Untrusted);
            }
        }
        EntryPoint::Process { .. } | EntryPoint::Ui { .. } => {
            if manifest.files.iter().any(|f| f.executable) {
                return Err(Error::Untrusted);
            }
        }
    }
    let mut dependencies = BTreeSet::new();
    for dependency in &manifest.dependencies {
        if dependency.package == manifest.package || !dependencies.insert(&dependency.package) {
            return Err(Error::Invalid("self or duplicate dependency".into()));
        }
    }
    if manifest
        .assets
        .iter()
        .map(|a| a.sha256)
        .collect::<BTreeSet<_>>()
        .len()
        != manifest.assets.len()
    {
        return Err(Error::Invalid("duplicate asset".into()));
    }
    Ok(())
}
fn hex(value: &str) -> Result<Vec<u8>> {
    if value.len() != 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Signature);
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or(Error::Signature)
        })
        .collect()
}

/// Verify a domain-bound message constructed by the owning artifact contract.
pub fn verify_detached_message(
    message: &[u8],
    signature_hex: &str,
    verifying_key: &[u8; 32],
) -> Result<()> {
    let raw: [u8; 64] = hex(signature_hex)?
        .try_into()
        .map_err(|_| Error::Signature)?;
    VerifyingKey::from_bytes(verifying_key)
        .map_err(|_| Error::Signature)?
        .verify_strict(message, &Signature::from_bytes(&raw))
        .map_err(|_| Error::Signature)
}
