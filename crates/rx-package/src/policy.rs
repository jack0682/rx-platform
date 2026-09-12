//! CLI trust input comes from an explicit local composition policy, never from the package being verified.
use crate::*;
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub id: Name,
    pub publisher: Name,
    pub verifying_key: Digest,
    pub kinds: BTreeSet<PackageKind>,
    pub permissions: BTreeSet<Permission>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub reference: ArtifactRef,
    pub path: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyInput {
    pub path: PathBuf,
    pub manifest_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_package_abis: Vec<Name>,
    pub schema: Name,
    pub contracts: ContractSet,
    pub target: Target,
    pub keys: Vec<Key>,
    pub assets: Vec<Asset>,
    pub dependencies: Vec<DependencyInput>,
}
pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    read_with_digest(path).map(|(value, _)| value)
}
pub fn read_with_digest<T: serde::de::DeserializeOwned>(path: &Path) -> Result<(T, Digest)> {
    let bytes = read_regular(path, 1_048_576)?;
    let value = canonical::decode_json(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok((value, content_digest(&bytes)))
}
fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let leaf = path
        .file_name()
        .ok_or_else(|| Error::Invalid("named input required".into()))?;
    let dir = cap_std::fs::Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(input_io)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = dir.open_with(leaf, &options).map_err(input_io)?;
    let meta = file.metadata().map_err(input_io)?;
    if !meta.is_file() || meta.len() > limit {
        return Err(Error::Invalid("input file type or size".into()));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(input_io)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Invalid("input too large".into()));
    }
    Ok(bytes)
}
fn input_io(_: std::io::Error) -> Error {
    Error::Invalid("policy input could not be acquired".into())
}
impl Policy {
    pub fn load(&self) -> Result<VerificationPolicy> {
        if !matches!(
            (
                self.schema.as_str(),
                self.additional_package_abis.is_empty()
            ),
            ("rx.package-verification-policy.v1", true)
                | ("rx.package-verification-policy.v2", false)
        ) || self.additional_package_abis.len() > 8
            || self
                .additional_package_abis
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.additional_package_abis.len()
            || self
                .additional_package_abis
                .contains(&self.contracts.package_abi)
            || self.keys.len() > 128
            || self.assets.len() > 1024
            || self.dependencies.len() > 128
        {
            return Err(Error::Invalid("verification policy shape".into()));
        }
        let mut publishers = BTreeMap::new();
        for key in &self.keys {
            if publishers
                .insert(
                    key.id.clone(),
                    TrustedPublisher {
                        publisher: key.publisher.clone(),
                        verifying_key: *key.verifying_key.as_bytes(),
                        kinds: key.kinds.clone(),
                        permissions: key.permissions.clone(),
                    },
                )
                .is_some()
            {
                return Err(Error::Invalid("duplicate policy key ID".into()));
            }
        }
        let mut assets = BTreeMap::new();
        let mut total = 0u64;
        for asset in &self.assets {
            total = total
                .checked_add(asset.reference.size_bytes.0)
                .ok_or_else(|| Error::Invalid("asset size overflow".into()))?;
            if total > 256 * 1024 * 1024 {
                return Err(Error::Invalid(
                    "policy asset acquisition exceeds 256 MiB".into(),
                ));
            }
            let data = read_regular(&asset.path, asset.reference.size_bytes.0)?;
            if data.len() as u64 != asset.reference.size_bytes.0
                || content_digest(&data) != asset.reference.sha256
                || assets
                    .insert(asset.reference.sha256, asset.reference.clone())
                    .is_some()
            {
                return Err(Error::Invalid(
                    "asset file digest or duplicate differs".into(),
                ));
            }
        }
        let mut policy = VerificationPolicy {
            additional_package_abis: self.additional_package_abis.iter().cloned().collect(),
            publishers,
            contracts: self.contracts.clone(),
            target: self.target.clone(),
            assets,
            dependencies: BTreeMap::new(),
            max_files: 32,
            max_content_bytes: 4 * 1024 * 1024,
        };
        let mut pending = BTreeMap::new();
        for input in &self.dependencies {
            let mut files = directory::acquire_directory(&input.path, 32, 4 * 1024 * 1024)?;
            let manifest = files
                .remove(&PackagePath::new("manifest.json").expect("literal"))
                .ok_or_else(|| Error::Invalid("dependency manifest missing".into()))?;
            let signature = files
                .remove(&PackagePath::new("manifest.sig.json").expect("literal"))
                .ok_or_else(|| Error::Invalid("dependency signature missing".into()))?;
            let metadata: Manifest =
                canonical::decode_json(&manifest).map_err(|e| Error::Invalid(e.to_string()))?;
            if content_digest(&manifest_bytes(&metadata)?) != input.manifest_digest
                || pending
                    .insert(
                        metadata.package.clone(),
                        (metadata, manifest, signature, files),
                    )
                    .is_some()
            {
                return Err(Error::Invalid(
                    "dependency identity or duplicate differs".into(),
                ));
            }
        }
        while !pending.is_empty() {
            let ready = pending
                .iter()
                .find(|(_, item)| {
                    item.0
                        .dependencies
                        .iter()
                        .all(|d| policy.dependencies.contains_key(&d.package))
                })
                .map(|(id, _)| id.clone())
                .ok_or_else(|| {
                    Error::Invalid("dependency cycle or unresolved dependency".into())
                })?;
            let (_, manifest, signature, files) =
                pending.remove(&ready).expect("selected dependency");
            let verified = verify_package(&manifest, &signature, files, &policy)?;
            policy.dependencies.insert(ready, Arc::new(verified));
        }
        Ok(policy)
    }
}
