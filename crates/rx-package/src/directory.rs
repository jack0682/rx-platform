//! Capability-rooted, bounded acquisition. VerifiedPackage owns the bytes, not source paths.
use crate::{Error, PackagePath, Result, VerificationPolicy, VerifiedPackage, verify_package};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{collections::BTreeMap, io::Read, path::Path};

pub fn verify_directory(path: &Path, policy: &VerificationPolicy) -> Result<VerifiedPackage> {
    let root = open_root(path)?;
    verify_open_directory(&root, policy)
}
/// All relative components must be real directories beneath a trusted import root.
pub fn verify_relative(
    root: &Path,
    relative: &PackagePath,
    policy: &VerificationPolicy,
) -> Result<VerifiedPackage> {
    let mut dir = open_root(root)?;
    for component in relative.as_str().split('/') {
        dir = dir.open_dir_nofollow(component).map_err(io_error)?;
    }
    verify_open_directory(&dir, policy)
}
pub(crate) fn verify_open_directory(
    root: &Dir,
    policy: &VerificationPolicy,
) -> Result<VerifiedPackage> {
    let mut files = acquire_open_directory(root, policy.max_files, policy.max_content_bytes)?;
    let manifest = files
        .remove(&PackagePath::new("manifest.json").expect("constant path"))
        .ok_or_else(|| Error::Invalid("manifest.json missing".into()))?;
    let signature = files
        .remove(&PackagePath::new("manifest.sig.json").expect("constant path"))
        .ok_or_else(|| Error::Invalid("manifest.sig.json missing".into()))?;
    verify_package(&manifest, &signature, files, policy)
}
/// Owned but UNVERIFIED directory bytes, for candidate/signature tooling.
/// This does not construct VerifiedPackage or grant trust.
#[derive(Clone, Copy)]
struct Limits {
    max_files: usize,
    max_content_bytes: u64,
}
pub fn acquire_directory(
    path: &Path,
    max_files: usize,
    max_content_bytes: u64,
) -> Result<BTreeMap<PackagePath, Vec<u8>>> {
    let root = open_root(path)?;
    acquire_open_directory(&root, max_files, max_content_bytes)
}
pub(crate) fn open_root(path: &Path) -> Result<Dir> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let leaf = path
        .file_name()
        .ok_or_else(|| Error::Invalid("named package directory required".into()))?;
    Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(io_error)?
        .open_dir_nofollow(leaf)
        .map_err(io_error)
}
pub(crate) fn acquire_open_directory(
    root: &Dir,
    max_files: usize,
    max_content_bytes: u64,
) -> Result<BTreeMap<PackagePath, Vec<u8>>> {
    if max_files == 0 || max_files > 4096 {
        return Err(Error::Invalid("acquisition file limit".into()));
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    let mut count = 0;
    read_tree(
        root,
        "",
        Limits {
            max_files,
            max_content_bytes,
        },
        &mut files,
        &mut total,
        &mut count,
        0,
    )?;
    Ok(files)
}
fn read_tree(
    dir: &Dir,
    prefix: &str,
    limits: Limits,
    files: &mut BTreeMap<PackagePath, Vec<u8>>,
    total: &mut u64,
    count: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > 24 {
        return Err(Error::Invalid("package directory depth".into()));
    }
    for entry in dir.entries().map_err(io_error)? {
        *count += 1;
        if *count > limits.max_files.saturating_mul(25).saturating_add(2) {
            return Err(Error::Invalid("directory entry limit".into()));
        }
        let entry = entry.map_err(io_error)?;
        let component = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Invalid("non-UTF8 filename".into()))?;
        let relative = if prefix.is_empty() {
            component.clone()
        } else {
            format!("{prefix}/{component}")
        };
        let path = PackagePath::new(relative.clone()).map_err(Error::Invalid)?;
        let kind = dir
            .symlink_metadata(&component)
            .map_err(io_error)?
            .file_type();
        if kind.is_symlink() {
            return Err(Error::Content(
                "symbolic links are not package files".into(),
            ));
        }
        if kind.is_dir() {
            let child = dir.open_dir_nofollow(&component).map_err(io_error)?;
            read_tree(&child, &relative, limits, files, total, count, depth + 1)?;
        } else if kind.is_file() {
            let remaining = limits
                .max_content_bytes
                .saturating_add(2 * 1024 * 1024)
                .saturating_sub(*total);
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
            }
            let file = dir.open_with(&component, &options).map_err(io_error)?;
            if !file.metadata().map_err(io_error)?.is_file() {
                return Err(Error::Content("regular file required".into()));
            }
            let mut bytes = Vec::new();
            file.take(remaining.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(io_error)?;
            if bytes.len() as u64 > remaining {
                return Err(Error::Invalid("acquisition size limit".into()));
            }
            *total += bytes.len() as u64;
            if files.len() >= limits.max_files.saturating_add(2) {
                return Err(Error::Invalid("acquisition file count limit".into()));
            }
            if files.insert(path, bytes).is_some() {
                return Err(Error::Content("duplicate acquired path".into()));
            }
        } else {
            return Err(Error::Content("regular package file required".into()));
        }
    }
    Ok(())
}
fn io_error(_: std::io::Error) -> Error {
    Error::Content("package directory could not be acquired".into())
}

/// Acquire an untrusted regular file beneath a trusted root, following no path component links.
pub fn read_relative_file(root: &Path, path: &PackagePath, max_bytes: u64) -> Result<Vec<u8>> {
    if max_bytes == 0 || max_bytes > 16 * 1024 * 1024 {
        return Err(Error::Invalid("artifact acquisition limit".into()));
    }
    let mut dir = open_root(root)?;
    let mut components = path.as_str().split('/').peekable();
    while let Some(component) = components.next() {
        if components.peek().is_some() {
            dir = dir.open_dir_nofollow(component).map_err(io_error)?;
        } else {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
            }
            let file = dir.open_with(component, &options).map_err(io_error)?;
            if !file.metadata().map_err(io_error)?.is_file() {
                return Err(Error::Invalid("regular artifact file required".into()));
            }
            let mut bytes = Vec::new();
            file.take(max_bytes + 1)
                .read_to_end(&mut bytes)
                .map_err(io_error)?;
            if bytes.len() as u64 > max_bytes {
                return Err(Error::Invalid("artifact size limit".into()));
            }
            return Ok(bytes);
        }
    }
    Err(Error::Invalid("artifact path empty".into()))
}

/// Publish generated immutable files without replacing an existing output. No signature/trust claim.
pub fn publish_files(files: BTreeMap<PackagePath, Vec<u8>>, out: &Path) -> Result<()> {
    publish_files_inner(files, out).map_err(|e| Error::Invalid(e.to_string()))
}
fn publish_files_inner(
    files: BTreeMap<PackagePath, Vec<u8>>,
    out: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
    };
    if out.exists() || fs::symlink_metadata(out).is_ok() {
        return Err("output already exists".into());
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(Error::Invalid("output parent must already exist".into()).into());
    }
    let staging = tempfile::Builder::new()
        .prefix(".rx-package-")
        .tempdir_in(parent)?;
    let mut directories = vec![staging.path().to_path_buf()];
    for (name, bytes) in files {
        let path = staging.path().join(name.as_str());
        let parent = path.parent().expect("package parent");
        fs::create_dir_all(parent)?;
        let mut ancestor = parent;
        while ancestor != staging.path() {
            directories.push(ancestor.to_path_buf());
            ancestor = ancestor.parent().expect("staging ancestor");
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    directories.sort();
    directories.dedup();
    for dir in directories.into_iter().rev() {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        out,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(Error::Invalid(
        "atomic package publication is currently supported on Linux and macOS".into(),
    )
    .into());
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
