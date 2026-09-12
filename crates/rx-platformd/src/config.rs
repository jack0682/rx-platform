use crate::Result;
use rx_api::auth::Credentials;
use rx_application::{CellConfiguration, Principal, Terminal};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    pub path: PathBuf,
    pub sha256: Digest,
}
impl PinnedFile {
    pub fn read(&self, secret: bool) -> Result<Vec<u8>> {
        if !self.path.is_absolute() {
            return Err("pinned path must be absolute".into());
        }
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1_048_576 {
            return Err("pinned file type/size invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if secret && metadata.permissions().mode() & 0o077 != 0 {
                return Err("secret file must not be group/world accessible".into());
            }
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)?
            .take(1_048_577)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576
            || Digest::from_bytes(Sha256::digest(&bytes).into()) != self.sha256
        {
            return Err("pinned file digest mismatch".into());
        }
        Ok(bytes)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    pub certificate: PinnedFile,
    pub key: PinnedFile,
    pub ca: PinnedFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Https {
    pub bind: SocketAddr,
    pub origin: String,
    pub tls: TlsFiles,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grpc {
    pub bind: SocketAddr,
    pub tls: TlsFiles,
    pub allowed_certificates: BTreeMap<Digest, Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostLink {
    pub host: Name,
    pub cell: Name,
    pub uri: String,
    pub server_name: String,
    pub server_fingerprint: Digest,
    pub tls: TlsFiles,
    pub ttl_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_intake: Option<PackageIntake>,
    pub schema: Name,
    #[serde(default)]
    pub host_links: Vec<HostLink>,
    pub installation_id: Id,
    pub release_digest: Digest,
    pub data_directory: PathBuf,
    pub runtime_directory: PathBuf,
    pub catalog: PinnedFile,
    pub credentials: PinnedFile,
    pub https: Https,
    pub grpc: Grpc,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageIntake {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_review_authority: Option<PinnedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualification_policy: Option<PinnedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_authority: Option<PinnedFile>,
    pub import_root: PathBuf,
    pub policy: PinnedFile,
}
pub struct LoadedIntake {
    pub device_review_authority: Option<PinnedFile>,
    pub qualification_policy: Option<PinnedFile>,
    pub review_authority: Option<PinnedFile>,
    pub policy_file: PathBuf,
    pub import_root: PathBuf,
    pub policy: rx_package::VerificationPolicy,
    pub file_digest: Digest,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: Name,
    pub bootstrap: Principal,
    pub principals: Vec<Principal>,
    pub terminals: Vec<Terminal>,
    pub cells: Vec<CellConfiguration>,
}
pub struct Loaded {
    pub package_intake: Option<LoadedIntake>,
    pub config: Config,
    pub catalog: Catalog,
    pub credentials: Credentials,
    pub https_tls: rx_api::terminal_https::TlsMaterial,
    pub grpc_tls: rx_api::grpc::TlsMaterial,
    pub host_links: Vec<rx_host_client::connection::ConnectionConfiguration>,
}
impl Loaded {
    pub fn read(path: &Path) -> Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
            return Err("invalid startup configuration file".into());
        }
        let config: Config = canonical::decode_json(&fs::read(path)?)?;
        if config.schema.as_str() != "rx.platform-startup.v1"
            || !config.data_directory.is_absolute()
            || !config.runtime_directory.is_absolute()
            || config.data_directory == config.runtime_directory
            || config.https.bind == config.grpc.bind
            || config.grpc.allowed_certificates.is_empty()
        {
            return Err("invalid startup configuration".into());
        }
        rx_api::terminal_https::HttpsPolicy::new(&config.https.origin)?;
        let catalog: Catalog = canonical::decode_json(&config.catalog.read(false)?)?;
        if catalog.schema.as_str() != "rx.platform-bootstrap-catalog.v1"
            || !catalog.bootstrap.active
            || !catalog
                .bootstrap
                .roles
                .contains(&rx_application::Role::AccountAdmin)
            || catalog.principals.len() > 256
            || catalog.terminals.len() > 256
            || catalog.cells.len() > 64
        {
            return Err("invalid bootstrap catalog".into());
        }
        let credentials: Credentials = canonical::decode_json(&config.credentials.read(true)?)?;
        let h = &config.https.tls;
        let g = &config.grpc.tls;
        if config.host_links.len() > 64
            || config
                .host_links
                .iter()
                .map(|h| (&h.host, &h.cell))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != config.host_links.len()
        {
            return Err("duplicate/oversized Host link configuration".into());
        }
        let mut host_links = vec![];
        for h in &config.host_links {
            if !h.uri.starts_with("https://") || h.ttl_ms.0 < 1000 || h.ttl_ms.0 > 30000 {
                return Err("Host link endpoint/lease configuration".into());
            }
            host_links.push(rx_host_client::connection::ConnectionConfiguration {
                host: h.host.clone(),
                cell: h.cell.clone(),
                server_pin: h.server_fingerprint,
                release: config.release_digest,
                ttl_ms: h.ttl_ms,
                endpoint: rx_host_client::TlsEndpoint {
                    uri: h.uri.clone(),
                    server_name: h.server_name.clone(),
                    server_ca_pem: h.tls.ca.read(false)?,
                    client_certificate_pem: h.tls.certificate.read(false)?,
                    client_key_pem: h.tls.key.read(true)?,
                },
            });
        }
        let package_intake = if let Some(input) = &config.package_intake {
            if !input.import_root.is_absolute()
                || input.import_root.starts_with(&config.data_directory)
                || config.data_directory.starts_with(&input.import_root)
            {
                return Err("package import root must be separate from authority data".into());
            }
            let m = fs::symlink_metadata(&input.import_root)?;
            if !m.is_dir() || m.file_type().is_symlink() {
                return Err("real package import directory required".into());
            }
            if !input.policy.path.is_absolute() {
                return Err("absolute package policy path required".into());
            }
            let (document, file_digest) = rx_package::policy::read_with_digest::<
                rx_package::policy::Policy,
            >(&input.policy.path)?;
            if file_digest != input.policy.sha256
                || document.assets.iter().any(|a| !a.path.is_absolute())
                || document.dependencies.iter().any(|d| !d.path.is_absolute())
            {
                return Err("package intake policy pin/paths differ".into());
            }
            if let Some(review) = &input.review_authority {
                let authority: rx_application::process_review::Authority =
                    canonical::decode_json(&review.read(false)?)?;
                authority.digest()?;
            }
            if let Some(source) = &input.device_review_authority {
                let authority: rx_application::device_review::Authority =
                    canonical::decode_json(&source.read(false)?)?;
                authority.digest()?;
            }
            if let Some(source) = &input.qualification_policy {
                let p: rx_application::requalification::Policy =
                    canonical::decode_json(&source.read(false)?)?;
                p.digest()?;
            }
            Some(LoadedIntake {
                device_review_authority: input.device_review_authority.clone(),
                qualification_policy: input.qualification_policy.clone(),
                review_authority: input.review_authority.clone(),
                policy_file: input.policy.path.clone(),
                import_root: input.import_root.clone(),
                policy: document.load()?,
                file_digest,
            })
        } else {
            None
        };
        let loaded = Self {
            package_intake,
            host_links,
            https_tls: rx_api::terminal_https::TlsMaterial {
                server_certificate_pem: h.certificate.read(false)?,
                server_key_pem: h.key.read(true)?,
                terminal_ca_pem: h.ca.read(false)?,
            },
            grpc_tls: rx_api::grpc::TlsMaterial {
                server_certificate_pem: g.certificate.read(false)?,
                server_key_pem: g.key.read(true)?,
                client_ca_pem: g.ca.read(false)?,
            },
            config,
            catalog,
            credentials,
        };
        rx_api::terminal_https::validate_material(loaded.https_tls.clone())?;
        rx_api::terminal_https::validate_material(rx_api::terminal_https::TlsMaterial {
            server_certificate_pem: loaded.grpc_tls.server_certificate_pem.clone(),
            server_key_pem: loaded.grpc_tls.server_key_pem.clone(),
            terminal_ca_pem: loaded.grpc_tls.client_ca_pem.clone(),
        })?;
        Ok(loaded)
    }
}
