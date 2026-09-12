//! Pinned S-owned browser assets, validated once and served from owned immutable bytes.
use crate::{
    error::ApiError,
    routes::exactly_one,
    terminal_https::{HttpsPolicy, TerminalPeer},
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use rx_domain::{
    canonical,
    types::{Counter, Digest},
};
use rx_package::{PackagePath, directory::read_relative_file};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::{Component, Path},
};

pub const SCHEMA: &str = "rx.operator-ui-bundle.v1";
pub const API_SCHEMA: &str = "rx.operator-api.v1";
pub const MANIFEST_FILENAME: &str = "operator-bundle.json";
const MAX_FILES: usize = 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    api_schema: String,
    files: Vec<FileEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    path: String,
    sha256: Digest,
    size_bytes: Counter,
}
#[derive(Clone)]
struct Asset {
    bytes: Bytes,
    media_type: &'static str,
}
/// Contains no live filesystem handles or request-selected paths.
#[derive(Clone)]
pub struct OperatorBundle {
    assets: BTreeMap<String, Asset>,
}
impl OperatorBundle {
    /// The expected manifest hash must come from the trusted deployment configuration.
    /// The release directory is mounted read-only; startup never trusts a self-declared hash.
    pub fn load(directory: &Path, manifest_path: &Path, expected: Digest) -> Result<Self, String> {
        real_directory(directory)?;
        if manifest_path != directory.join(MANIFEST_FILENAME) {
            return Err(
                "operator manifest must be operator-bundle.json in the bundle directory".into(),
            );
        }
        let bytes = read_file(directory, MANIFEST_FILENAME, MAX_MANIFEST_BYTES)?;
        if hash(&bytes) != expected {
            return Err("operator manifest digest mismatch".into());
        }
        let manifest: Manifest = canonical::decode_json(&bytes).map_err(|e| e.to_string())?;
        validate_manifest(&manifest)?;
        let expected_paths: BTreeSet<_> = manifest.files.iter().map(|f| f.path.clone()).collect();
        if inventory(directory)? != expected_paths {
            return Err("operator bundle contains missing or unlisted files".into());
        }
        let mut assets = BTreeMap::new();
        for file in manifest.files {
            let bytes = read_file(directory, &file.path, file.size_bytes.0)?;
            if bytes.len() as u64 != file.size_bytes.0 || hash(&bytes) != file.sha256 {
                return Err("operator asset size or digest mismatch".into());
            }
            let media_type = media_type(&file.path).ok_or("operator asset type unsupported")?;
            assets.insert(
                file.path,
                Asset {
                    bytes: Bytes::from(bytes),
                    media_type,
                },
            );
        }
        Ok(Self { assets })
    }

    pub(crate) fn router(self, policy: HttpsPolicy) -> Router {
        // Only exact known assets are routes. Missing assets and /api retain the API's JSON fallback.
        let mut router = Router::new();
        for (path, asset) in self.assets {
            if path == "index.html" {
                router = add_asset(router, "/".into(), asset.clone());
            }
            router = add_asset(router, format!("/{path}"), asset);
        }
        router.layer(middleware::from_fn_with_state(policy, ingress))
    }
}
fn hash(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}
fn real_directory(directory: &Path) -> Result<(), String> {
    if !directory.is_absolute()
        || directory
            .components()
            .any(|p| !matches!(p, Component::RootDir | Component::Normal(_)))
        || fs::canonicalize(directory).map_err(|e| e.to_string())? != directory
    {
        return Err(
            "operator directory must be absolute and contain no symlink or traversal".into(),
        );
    }
    let metadata = fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("operator directory must be a real directory".into());
    }
    Ok(())
}
fn read_file(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
    // The inventory is not authority for a later open: acquire beneath the directory capability,
    // with nofollow, NONBLOCK and regular-file validation on the actually opened descriptor.
    let path = PackagePath::new(relative)?;
    read_relative_file(root, &path, limit).map_err(|e| e.to_string())
}
fn valid_relative(path: &str) -> bool {
    PackagePath::new(path).is_ok() && path.split('/').count() <= 16
}
fn media_type(path: &str) -> Option<&'static str> {
    if path == "index.html" {
        return Some("text/html; charset=utf-8");
    }
    if !valid_relative(path) || !path.starts_with("assets/") {
        return None;
    }
    Some(match path.rsplit_once('.')?.1 {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "json" => "application/json",
        _ => return None,
    })
}
fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != SCHEMA
        || manifest.api_schema != API_SCHEMA
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
        || !manifest
            .files
            .iter()
            .any(|f| f.path == "index.html" && f.size_bytes.0 > 0)
        || !manifest
            .files
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    {
        return Err("operator manifest schema, count, index or sorted paths invalid".into());
    }
    let mut total = 0u64;
    for file in &manifest.files {
        if media_type(&file.path).is_none()
            || file.size_bytes.0 == 0
            || file.size_bytes.0 > MAX_FILE_BYTES
        {
            return Err("operator asset path, type or size invalid".into());
        }
        total = total
            .checked_add(file.size_bytes.0)
            .ok_or("operator bundle size overflow")?;
        if total > MAX_TOTAL_BYTES {
            return Err("operator bundle exceeds total size limit".into());
        }
    }
    Ok(())
}
fn inventory(directory: &Path) -> Result<BTreeSet<String>, String> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = BTreeSet::new();
    let mut entries = 0usize;
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(current).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            entries += 1;
            if entries > MAX_FILES * 2 + 1 {
                return Err("operator directory entry limit exceeded".into());
            }
            let relative = path
                .strip_prefix(directory)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("operator path must be ASCII")?
                .to_owned();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                return Err("operator symlinks are forbidden".into());
            }
            if metadata.is_dir() {
                if !valid_relative(&relative)
                    || !(relative == "assets" || relative.starts_with("assets/"))
                {
                    return Err("operator subdirectory outside assets".into());
                }
                pending.push(path);
            } else if metadata.is_file() {
                if relative != MANIFEST_FILENAME {
                    files.insert(relative);
                }
            } else {
                return Err("operator entry is not a regular file or directory".into());
            }
        }
    }
    Ok(files)
}
fn add_asset(router: Router, path: String, asset: Asset) -> Router {
    let expected = path.clone();
    router.route(
        &path,
        get(move |request: Request| {
            let asset = asset.clone();
            let expected = expected.clone();
            async move {
                if request.uri().path() != expected {
                    return ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND").into_response();
                }
                let mut response = Response::new(if request.method() == Method::HEAD {
                    Body::empty()
                } else {
                    Body::from(asset.bytes.clone())
                });
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(asset.media_type),
                );
                response
                    .headers_mut()
                    .insert(header::CONTENT_LENGTH, HeaderValue::from(asset.bytes.len()));
                response
            }
        }),
    )
}
async fn ingress(State(policy): State<HttpsPolicy>, request: Request, next: Next) -> Response {
    let allowed = request.extensions().get::<TerminalPeer>().is_some()
        && request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some()
        && exactly_one(request.headers(), "host") == Some(policy.host.as_str())
        && (!request.headers().contains_key(header::ORIGIN)
            || exactly_one(request.headers(), "origin") == Some(policy.origin.as_str()));
    let mut response = if !allowed {
        ApiError::forbidden().into_response()
    } else if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let mut response =
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "METHOD_NOT_ALLOWED").into_response();
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        response
    } else {
        // Static content does not create or resolve human sessions. Those remain under /api.
        next.run(request).await
    };
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::PRAGMA, "no-cache"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod tests;
