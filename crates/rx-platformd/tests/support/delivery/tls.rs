use super::*;
use rcgen::*;
use rx_platformd::config::{PinnedFile, TlsFiles};

pub(super) struct Material {
    pub files: TlsFiles,
    pub fingerprint: Digest,
}
pub(super) struct Bundle {
    pub p_server: Material,
    pub p_client: Material,
    pub h_server: Material,
    pub publisher: Material,
    pub executor: Material,
    pub terminal_fingerprint: Digest,
}
fn leaf(
    issuer: &CertifiedIssuer<'_, KeyPair>,
    out: &Path,
    role: (&str, &str),
    label: &str,
    server: bool,
    sans: &[&str],
    ca_hash: Digest,
) -> Result<Material> {
    let (role_dir, runtime_root) = role;
    let key = KeyPair::generate()?;
    let mut params =
        CertificateParams::new(sans.iter().map(|v| v.to_string()).collect::<Vec<_>>())?;
    params.distinguished_name.push(
        DnType::CommonName,
        format!("RX SIMULATION delivery {label}"),
    );
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.use_authority_key_identifier_extension = true;
    params.extended_key_usages = vec![if server {
        ExtendedKeyUsagePurpose::ServerAuth
    } else {
        ExtendedKeyUsagePurpose::ClientAuth
    }];
    let certificate = params.signed_by(&key, issuer)?;
    let cert_data = certificate.pem().into_bytes();
    let key_data = key.serialize_pem().into_bytes();
    write(
        &out.join(role_dir).join("pki").join(format!("{label}.pem")),
        &cert_data,
    )?;
    write(
        &out.join(role_dir).join("pki").join(format!("{label}.key")),
        &key_data,
    )?;
    Ok(Material {
        files: TlsFiles {
            certificate: PinnedFile {
                path: PathBuf::from(format!("{runtime_root}/pki/{label}.pem")),
                sha256: rx_package::content_digest(&cert_data),
            },
            key: PinnedFile {
                path: PathBuf::from(format!("{runtime_root}/pki/{label}.key")),
                sha256: rx_package::content_digest(&key_data),
            },
            ca: PinnedFile {
                path: PathBuf::from(format!("{runtime_root}/pki/ca.pem")),
                sha256: ca_hash,
            },
        },
        fingerprint: rx_package::content_digest(certificate.der().as_ref()),
    })
}
pub(super) fn export(out: &Path) -> Result<Bundle> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params
        .distinguished_name
        .push(DnType::CommonName, "RX disposable SIMULATION delivery CA");
    let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate()?)?;
    let ca = issuer.pem().into_bytes();
    let ca_hash = rx_package::content_digest(&ca);
    for dir in ["config", "host-config", "executor-config", "browser"] {
        write(&out.join(dir).join("pki/ca.pem"), &ca)?;
    }
    let p_server = leaf(
        &issuer,
        out,
        ("config", "/config"),
        "p-server",
        true,
        &["p", "localhost", "127.0.0.1"],
        ca_hash,
    )?;
    let p_client = leaf(
        &issuer,
        out,
        ("config", "/config"),
        "p-host-client",
        false,
        &["platform"],
        ca_hash,
    )?;
    let h_server = leaf(
        &issuer,
        out,
        ("host-config", "/config/host"),
        "h-server",
        true,
        &["s", "localhost", "127.0.0.1"],
        ca_hash,
    )?;
    let publisher = leaf(
        &issuer,
        out,
        ("host-config", "/config/host"),
        "h-publisher",
        false,
        &["host-sim"],
        ca_hash,
    )?;
    let executor = leaf(
        &issuer,
        out,
        ("executor-config", "/config/executor"),
        "executor-client",
        false,
        &["executor"],
        ca_hash,
    )?;
    let terminal = leaf(
        &issuer,
        out,
        ("browser", "/outside-containers"),
        "terminal",
        false,
        &["panel-sim"],
        ca_hash,
    )?;
    // The issuer's private key is intentionally never written.
    Ok(Bundle {
        p_server,
        p_client,
        h_server,
        publisher,
        executor,
        terminal_fingerprint: terminal.fingerprint,
    })
}
