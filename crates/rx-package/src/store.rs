//! Owned package bytes only. Neither storage nor successful verification grants activation.
use crate::{
    Error, PackagePath, Result, VerificationPolicy, VerifiedPackage, content_digest, directory,
    manifest_bytes,
};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectId {
    pub manifest: Digest,
    pub signature: Digest,
}
impl ObjectId {
    fn directory_name(&self) -> String {
        format!("{}-{}", self.manifest, self.signature)
    }
}
/// One owner, anchored directory handles, no source paths retained after verification.
/// Root and its parent must be administrator-owned composition paths, not web input.
pub struct Store {
    owner: Id,
    root: Dir,
    _ownership: std::fs::File,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_mode(path, true)
    }
    pub fn open_existing(path: &Path) -> Result<Self> {
        Self::open_mode(path, false)
    }
    fn open_mode(path: &Path, initialize: bool) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::Invalid("package store root must be absolute".into()));
        }
        let leaf = path
            .file_name()
            .ok_or_else(|| Error::Invalid("named store root required".into()))?;
        let parent = Dir::open_ambient_dir(
            path.parent().expect("absolute named path"),
            cap_std::ambient_authority(),
        )
        .map_err(io)?;
        if initialize {
            match parent.create_dir(leaf) {
                Ok(()) => sync(&parent)?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(io(e)),
            }
        }
        let root = parent.open_dir_nofollow(leaf).map_err(io)?;
        let mut options = regular_options();
        options
            .read(true)
            .write(true)
            .create(initialize)
            .truncate(false);
        let ownership = root
            .open_with("store.lock", &options)
            .map_err(io)?
            .into_std();
        if !ownership.metadata().map_err(io)?.is_file() {
            return Err(Error::Invalid("package store lock must be regular".into()));
        }
        ownership
            .try_lock()
            .map_err(|_| Error::Invalid("package store already owned".into()))?;
        match root.symlink_metadata("store.json") {
            Ok(_) => {
                let mut options = regular_options();
                options.read(true);
                let f = root.open_with("store.json", &options).map_err(io)?;
                if !f.metadata().map_err(io)?.is_file() {
                    return Err(Error::Invalid(
                        "package store marker must be regular".into(),
                    ));
                }
                let mut bytes = Vec::new();
                f.take(1025).read_to_end(&mut bytes).map_err(io)?;
                if bytes.len() > 1024
                    || canonical::decode_json::<serde_json::Value>(&bytes).ok()
                        != Some(serde_json::json!({"schema":"rx.package-store.v1"}))
                {
                    return Err(Error::Invalid("package store marker differs".into()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && initialize => {
                for entry in root.entries().map_err(io)? {
                    if entry.map_err(io)?.file_name() != "store.lock" {
                        return Err(Error::Invalid(
                            "refusing a nonempty uninitialized package store".into(),
                        ));
                    }
                }
                write_new(
                    &root,
                    "store.pending",
                    b"{\"schema\":\"rx.package-store.v1\"}",
                )?;
                publish(&root, "store.pending", "store.json")?;
            }
            Err(e) => return Err(io(e)),
        }
        sync(&root)?;
        Ok(Self {
            owner: Id::new(uuid::Uuid::new_v4().to_string()).expect("uuid"),
            root,
            _ownership: ownership,
        })
    }
    pub fn owner(&self) -> &Id {
        &self.owner
    }
    /// Only this store can create this receipt, after reacquiring its stored bytes.
    pub fn verify_owned(
        &self,
        object: &ObjectId,
        policy: &VerificationPolicy,
    ) -> Result<StoredPackage> {
        Ok(StoredPackage {
            owner: self.owner.clone(),
            object: object.clone(),
            policy_fingerprint: policy.fingerprint()?,
            package: self.verify(object, policy)?,
        })
    }
    pub fn put(&mut self, package: &VerifiedPackage) -> Result<ObjectId> {
        let signature =
            canonical::bytes(package.signature()).map_err(|e| Error::Invalid(e.to_string()))?;
        let object = ObjectId {
            manifest: package.digest(),
            signature: content_digest(&signature),
        };
        let mut files: BTreeMap<_, _> = package
            .files()
            .map(|(p, b)| (p.clone(), b.to_vec()))
            .collect();
        files.insert(
            PackagePath::new("manifest.json").expect("literal"),
            manifest_bytes(package.manifest())?,
        );
        files.insert(
            PackagePath::new("manifest.sig.json").expect("literal"),
            signature,
        );
        let target = object.directory_name();
        match self.root.symlink_metadata(&target) {
            Ok(_) => {
                let dir = self.root.open_dir_nofollow(&target).map_err(io)?;
                let old = directory::acquire_open_directory(
                    &dir,
                    package.manifest().files.len().max(1),
                    files.values().map(|b| b.len() as u64).sum(),
                )?;
                if old != files {
                    return Err(Error::Content(
                        "stored object differs; repair must be explicit".into(),
                    ));
                }
                // Retry after publication/root-sync failure must also sync before success.
                sync(&self.root)?;
                return Ok(object);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(io(e)),
        }
        let staging = format!(".incoming-{}", uuid::Uuid::new_v4());
        self.root.create_dir(&staging).map_err(io)?;
        let dir = self.root.open_dir_nofollow(&staging).map_err(io)?;
        let mut directories = std::collections::BTreeSet::new();
        for (path, bytes) in files {
            let path = Path::new(path.as_str());
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                dir.create_dir_all(parent).map_err(io)?;
                for ancestor in parent.ancestors().filter(|p| !p.as_os_str().is_empty()) {
                    directories.insert(ancestor.to_path_buf());
                }
            }
            write_new(&dir, path, &bytes)?;
        }
        for path in directories.into_iter().rev() {
            sync(&dir.open_dir_nofollow(path).map_err(io)?)?;
        }
        sync(&dir)?;
        // Crash/error may leave an unpublished .incoming directory. Never auto-adopt it.
        publish(&self.root, &staging, &target)?;
        sync(&self.root)?;
        Ok(object)
    }
    /// Fresh byte acquisition and CURRENT policy; prior storage is not trust.
    pub fn verify(
        &self,
        object: &ObjectId,
        policy: &VerificationPolicy,
    ) -> Result<VerifiedPackage> {
        let dir = self
            .root
            .open_dir_nofollow(object.directory_name())
            .map_err(io)?;
        let package = directory::verify_open_directory(&dir, policy)?;
        let signature = content_digest(
            &canonical::bytes(package.signature()).map_err(|e| Error::Invalid(e.to_string()))?,
        );
        if package.digest() != object.manifest || signature != object.signature {
            return Err(Error::Content("stored object identity differs".into()));
        }
        Ok(package)
    }
}
fn regular_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
        options.mode(0o600);
    }
    options
}
fn write_new(dir: &Dir, path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let mut options = regular_options();
    options.write(true).create_new(true);
    let mut file = dir.open_with(path, &options).map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn sync(dir: &Dir) -> Result<()> {
    // cap-std may retain O_PATH on Linux; that capability cannot itself be fsynced.
    // Reopen this same directory relative to its handle, without resolving an ambient path.
    #[cfg(unix)]
    {
        let fd = rustix::fs::openat(
            dir,
            ".",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|e| io(e.into()))?;
        rustix::fs::fsync(&fd).map_err(|e| io(e.into()))
    }
    #[cfg(not(unix))]
    {
        dir.try_clone()
            .map_err(io)?
            .into_std_file()
            .sync_all()
            .map_err(io)
    }
}
fn publish(dir: &Dir, from: &str, to: &str) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::fs::renameat_with(dir, from, dir, to, rustix::fs::RenameFlags::NOREPLACE)
            .map_err(|e| io(e.into()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (dir, from, to);
        Err(Error::Invalid(
            "package publication requires Linux or macOS".into(),
        ))
    }
}
fn io(error: std::io::Error) -> Error {
    Error::Content(format!(
        "package object store I/O failed ({})",
        error.kind()
    ))
}

/// In-process evidence of acquisition from one registered store owner, not a wire PASS assertion.
pub struct StoredPackage {
    owner: Id,
    object: ObjectId,
    policy_fingerprint: Digest,
    package: VerifiedPackage,
}
impl StoredPackage {
    pub fn owner(&self) -> &Id {
        &self.owner
    }
    pub fn object(&self) -> &ObjectId {
        &self.object
    }
    pub fn policy_fingerprint(&self) -> Digest {
        self.policy_fingerprint
    }
    pub fn package(&self) -> &VerifiedPackage {
        &self.package
    }
}
