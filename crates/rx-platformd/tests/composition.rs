mod support;
use rx_application::*;
use rx_domain::{canonical, types::*};
use rx_platformd::{config::*, *};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "composition-test".into(),
            ticks_ns: Counter(1000),
        }
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    config: Config,
    client: reqwest::Client,
    peer_pem: String,
    peer_key: String,
    ca_pem: String,
}
fn pin(dir: &Path, label: &str, bytes: &[u8]) -> PinnedFile {
    let path = dir.join(label);
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    PinnedFile {
        path,
        sha256: Digest::from_bytes(Sha256::digest(bytes).into()),
    }
}
fn fixture() -> Fixture {
    use rcgen::*;
    let dir = tempfile::tempdir().unwrap();
    let mut p = CertificateParams::new(vec![]).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    p.distinguished_name
        .push(DnType::CommonName, "RX isolated test CA");
    let ca = CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap();
    let leaf = |server: bool| {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        p.distinguished_name.push(
            DnType::CommonName,
            if server {
                "RX isolated test server"
            } else {
                "RX isolated test client"
            },
        );
        p.use_authority_key_identifier_extension = true;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![if server {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        (p.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf(true);
    let (terminal, terminal_key) = leaf(false);
    let (peer, peer_key) = leaf(false);
    let fingerprint = Digest::from_bytes(Sha256::digest(terminal.der().as_ref()).into());
    let tls = TlsFiles {
        certificate: pin(dir.path(), "server.pem", server.pem().as_bytes()),
        key: pin(
            dir.path(),
            "server.key",
            server_key.serialize_pem().as_bytes(),
        ),
        ca: pin(dir.path(), "ca.pem", ca.pem().as_bytes()),
    };
    let p = Principal {
        id: name("admin"),
        client_namespace: name("admin"),
        roles: [
            Role::AccountAdmin,
            Role::Engineer,
            Role::Operator,
            Role::Observer,
        ]
        .into_iter()
        .collect(),
        cells: [name("cell/a")].into_iter().collect(),
        active: true,
    };
    let catalog = Catalog {
        schema: name("rx.platform-bootstrap-catalog.v1"),
        bootstrap: p,
        principals: vec![Principal {
            id: name("operator-api"),
            client_namespace: name("operator-api"),
            roles: [Role::OperatorApi, Role::Observer].into_iter().collect(),
            cells: [name("cell/a")].into_iter().collect(),
            active: true,
        }],
        terminals: vec![Terminal {
            id: name("panel/a"),
            certificate_digest: fingerprint,
            cells: [name("cell/a")].into_iter().collect(),
            active: true,
        }],
        cells: vec![support::configuration("cell/a")],
    };
    let credentials = json!({"schema":"rx.local-credentials.v1","accounts":[{"principal":"admin","password_hash":rx_api::auth::password_hash("composition-test-password").unwrap()}]});
    let h = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let g = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let config = Config {
        package_intake: None,
        host_links: vec![HostLink {
            host: name("host/sim"),
            cell: name("cell/a"),
            uri: "https://127.0.0.1:9".into(),
            server_name: "localhost".into(),
            server_fingerprint: Digest::from_bytes([90; 32]),
            tls: TlsFiles {
                certificate: pin(dir.path(), "host-client.pem", peer.pem().as_bytes()),
                key: pin(
                    dir.path(),
                    "host-client.key",
                    peer_key.serialize_pem().as_bytes(),
                ),
                ca: tls.ca.clone(),
            },
            ttl_ms: Counter(1000),
        }],
        schema: name("rx.platform-startup.v1"),
        installation_id: id(),
        release_digest: Digest::from_bytes([8; 32]),
        data_directory: dir.path().join("data"),
        runtime_directory: dir.path().join("run"),
        catalog: pin(
            dir.path(),
            "catalog.json",
            &canonical::bytes(&catalog).unwrap(),
        ),
        credentials: pin(
            dir.path(),
            "credentials.json",
            &canonical::bytes(&credentials).unwrap(),
        ),
        https: Https {
            bind: h.local_addr().unwrap(),
            origin: format!("https://{}", h.local_addr().unwrap()),
            tls: tls.clone(),
        },
        grpc: Grpc {
            bind: g.local_addr().unwrap(),
            tls,
            allowed_certificates: BTreeMap::from([(
                Digest::from_bytes(Sha256::digest(peer.der().as_ref()).into()),
                name("operator-api"),
            )]),
        },
    };
    fs::create_dir(&config.runtime_directory).unwrap();
    let path = dir.path().join("startup.json");
    fs::write(&path, canonical::bytes(&config).unwrap()).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_pem(ca.pem().as_bytes()).unwrap())
        .identity(
            reqwest::Identity::from_pem(
                format!("{}{}", terminal.pem(), terminal_key.serialize_pem()).as_bytes(),
            )
            .unwrap(),
        )
        .build()
        .unwrap();
    Fixture {
        _dir: dir,
        path,
        config,
        client,
        peer_pem: peer.pem(),
        peer_key: peer_key.serialize_pem(),
        ca_pem: ca.pem(),
    }
}
async fn wait_ready(f: &Fixture) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(bytes) = fs::read(f.config.runtime_directory.join("platform-status.json")) {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            if value["phase"] == "SOFTWARE_READY_UNCOMMISSIONED" {
                return value;
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "startup timeout");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test]
async fn composed_startup_restart_and_stop_preserve_store_and_never_start_a_run() {
    let f = fixture();
    initialize(&f.path).unwrap();
    assert!(initialize(&f.path).is_err());
    let mut previous_boot = None;
    for iteration in 0..2 {
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let path = f.path.clone();
        let task = tokio::spawn(async move {
            serve(&path, TestClock, async {
                let _ = stopped.await;
            })
            .await
        });
        let status = wait_ready(&f).await;
        let duplicate = serve(&f.path, TestClock, std::future::pending()).await;
        assert!(duplicate.is_err());
        let preserved: Value = serde_json::from_slice(
            &fs::read(f.config.runtime_directory.join("platform-status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(preserved["runtime_boot"], status["runtime_boot"]);
        assert_eq!(preserved["phase"], "SOFTWARE_READY_UNCOMMISSIONED");
        assert_ne!(previous_boot.as_ref(), Some(&status["runtime_boot"]));
        previous_boot = Some(status["runtime_boot"].clone());
        let response = f
            .client
            .post(format!("{}/api/v1/session", f.config.https.origin))
            .header("origin", &f.config.https.origin)
            .header("x-rx-client", "browser-v1")
            .json(&json!({"principal":"admin","password":"composition-test-password"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let profile: Value = response.json().await.unwrap();
        if iteration == 1 {
            assert!(
                !profile["roles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r == "ACCOUNT_ADMIN")
            );
        }
        let overview: Value = f
            .client
            .get(format!("{}/api/v1/overview", f.config.https.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(overview["cells"][0]["runs"].as_array().unwrap().is_empty());
        use rx_protocol::{base, cell};
        let channel =
            tonic::transport::Channel::from_shared(format!("https://{}", f.config.grpc.bind))
                .unwrap()
                .tls_config(
                    tonic::transport::ClientTlsConfig::new()
                        .domain_name("localhost")
                        .ca_certificate(tonic::transport::Certificate::from_pem(&f.ca_pem))
                        .identity(tonic::transport::Identity::from_pem(
                            &f.peer_pem,
                            &f.peer_key,
                        )),
                )
                .unwrap()
                .connect()
                .await
                .unwrap();
        let manifest = |raw: &str| {
            let v: Value = serde_json::from_str(raw).unwrap();
            Sha256::digest(canonical::bytes(&v).unwrap()).to_vec()
        };
        let base_hash = manifest(include_str!(
            "../../../spec/contracts/v1.0/protocol_manifest.json"
        ));
        let cell_hash = manifest(include_str!(
            "../../../spec/cell_operations/v1.0/protocol_manifest.json"
        ));
        let session = base::session_service_client::SessionServiceClient::new(channel.clone())
            .open(base::PeerHello {
                peer_id: "operator-api".into(),
                role: base::Role::OperatorApi as i32,
                boot_id: id().to_string(),
                installation_id: f.config.installation_id.to_string(),
                store_generation: overview["installation"]["store_generation"]
                    .as_str()
                    .unwrap()
                    .into(),
                supported_versions: vec![base::Version {
                    major: 1,
                    minor: 0,
                    schema_hash: base_hash.clone(),
                }],
                release_digest: f.config.release_digest.as_bytes().to_vec(),
                journal_id: None,
                last_seq: None,
                shared_clock_id: "composition-test".into(),
            })
            .await
            .unwrap()
            .into_inner();
        let mut cells = cell::cell_service_client::CellServiceClient::new(channel);
        cells
            .open(cell::CellHello {
                base_manifest_hash: base_hash,
                cell_manifest_hash: cell_hash,
                peer_id: "operator-api".into(),
                base_session_id: session.session_id.clone(),
                cell_definition_digest: support::configuration("cell/a")
                    .definition
                    .sha256
                    .as_bytes()
                    .to_vec(),
                shared_clock_id: "composition-test".into(),
            })
            .await
            .unwrap();
        let context = cells
            .inspect(cell::CellCall {
                context: Some(base::CallContext {
                    session_id: session.session_id,
                    call_id: id().to_string(),
                    request_key: None,
                    expected_revision: None,
                }),
                cell_id: "cell/a".into(),
                expected_cell_revision: None,
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            context.commissioning,
            cell::Commissioning::NotCommissioned as i32
        );
        drop(cells);

        assert!(overview["cells"][0]["cell"]["value"]["qualification"].is_null());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let current: Value = f
                .client
                .get(format!("{}/api/v1/overview", f.config.https.origin))
                .header(reqwest::header::COOKIE, &cookie)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let health = &current["cells"][0]["diagnostics"]["hosts"][0]["runtime"];
            if health["availability"] == "FRESH" && health["connection"] == "WAITING_FOR_PEER" {
                assert_eq!(health["observation"], "WAITING");
                assert_eq!(health["delivery"], "NOT_STARTED");
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "service telemetry timeout: {health}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        stop.send(()).unwrap();
        let report = tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(report.lifecycle.phase, lifecycle::Phase::StopCommitted);
        assert!(!report.physical_shutdown_assessed);
        assert_eq!(report.attention_count, Counter(0));
        if iteration == 0 {
            use rx_ports::Repository;
            let mut store =
                rx_storage::SqliteRepository::open(f.config.data_directory.join("platform.db"))
                    .unwrap();
            store
                .transact(|tx| {
                    let key = rx_application::persistence::key("principal", name("admin"));
                    let row = tx.get(&key)?.unwrap();
                    let mut principal: Principal =
                        rx_application::persistence::decode(&row, "rx.internal.principal.v1")?;
                    principal.roles.remove(&Role::AccountAdmin);
                    tx.put(
                        &key,
                        Some(row.revision),
                        &rx_application::persistence::doc("rx.internal.principal.v1", &principal)?,
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        assert!(
            f.client
                .get(format!("{}/api/v1/health", f.config.https.origin))
                .send()
                .await
                .is_err()
        );
    }
}
#[test]
fn invalid_pins_or_catalog_never_publish_a_partial_installation() {
    let f = fixture();
    fs::write(&f.config.catalog.path, b"{}").unwrap();
    assert!(initialize(&f.path).is_err());
    assert!(!f.config.data_directory.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_executable_uses_shared_clock_and_sigterm_commits_process_stop() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let f = fixture();
    let initialized = std::process::Command::new(env!("CARGO_BIN_EXE_rx-platformd"))
        .arg("init")
        .arg(&f.path)
        .output()
        .unwrap();
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let mut child = Child(
        std::process::Command::new(env!("CARGO_BIN_EXE_rx-platformd"))
            .arg("run")
            .arg(&f.path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let ready = wait_ready(&f).await;
    assert!(child.0.try_wait().unwrap().is_none());
    let login = f
        .client
        .post(format!("{}/api/v1/session", f.config.https.origin))
        .header("origin", &f.config.https.origin)
        .header("x-rx-client", "browser-v1")
        .json(&json!({"principal":"admin","password":"composition-test-password"}))
        .send()
        .await
        .unwrap();
    let profile: Value = login.json().await.unwrap();
    assert!(
        profile["expires_at"]["clock_id"]
            .as_str()
            .unwrap()
            .starts_with("linux-boottime/")
    );
    assert!(
        std::process::Command::new("/bin/kill")
            .arg("-TERM")
            .arg(child.0.id().to_string())
            .status()
            .unwrap()
            .success()
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(exit) = child.0.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "SIGTERM exit timeout"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let stopped: Value = serde_json::from_slice(
        &fs::read(f.config.runtime_directory.join("platform-status.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(stopped["runtime_boot"], ready["runtime_boot"]);
    assert_eq!(stopped["phase"], "PROCESS_STOPPED");
    assert_eq!(stopped["stop"]["lifecycle"]["phase"], "STOP_COMMITTED");
    assert_eq!(stopped["physical_shutdown_assessed"], false);
}

#[test]
#[ignore = "tools/test_platform_image.py requests isolated temporary container inputs"]
fn export_container_fixture() {
    let output =
        PathBuf::from(std::env::var("RX_PLATFORM_IMAGE_FIXTURE").expect("fixture directory"));
    fs::create_dir(&output).unwrap();
    let f = fixture();
    let mut c = f.config.clone();
    let relocate = |file: &mut PinnedFile| {
        let name = file.path.file_name().unwrap();
        fs::copy(&file.path, output.join(name)).unwrap();
        file.path = Path::new("/config").join(name);
    };
    relocate(&mut c.catalog);
    relocate(&mut c.credentials);
    for tls in [&mut c.https.tls, &mut c.grpc.tls] {
        relocate(&mut tls.certificate);
        relocate(&mut tls.key);
        relocate(&mut tls.ca);
    }
    for link in &mut c.host_links {
        relocate(&mut link.tls.certificate);
        relocate(&mut link.tls.key);
        relocate(&mut link.tls.ca);
    }
    let port = std::env::var("RX_PLATFORM_IMAGE_PORT").unwrap();
    c.https.bind = "0.0.0.0:8443".parse().unwrap();
    c.https.origin = format!("https://127.0.0.1:{port}");
    c.grpc.bind = "0.0.0.0:7443".parse().unwrap();
    c.data_directory = PathBuf::from("/data/install");
    c.runtime_directory = PathBuf::from("/data/runtime");
    fs::write(output.join("startup.json"), canonical::bytes(&c).unwrap()).unwrap();
    fs::write(output.join("probe.pem"), f.peer_pem).unwrap();
    fs::write(output.join("probe.key"), f.peer_key).unwrap();
}

#[path = "../../rx-application/tests/support/package_intake.rs"]
mod intake_support;
#[tokio::test]
async fn configured_package_intake_uses_real_terminal_https_and_retains_history_across_restart() {
    let mut f = fixture();
    let p = intake_support::fixture();
    let input_root = p.import_root.clone();
    let object = p.object.clone();
    // The separately prepared store is not copied: the daemon imports into its own data directory.
    drop(p.store);
    let policy_pin = pin(f._dir.path(), "package-policy.json", &p.policy_bytes);
    let _ = p.policy.fingerprint().unwrap();
    f.config.package_intake = Some(PackageIntake {
        device_review_authority: None,
        qualification_policy: None,
        review_authority: None,
        import_root: input_root,
        policy: policy_pin,
    });
    fs::write(&f.path, canonical::bytes(&f.config).unwrap()).unwrap();
    initialize(&f.path).unwrap();
    let mut original = None;
    let mut body = None;
    for iteration in 0..2 {
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let path = f.path.clone();
        let task = tokio::spawn(async move {
            serve(&path, TestClock, async {
                let _ = stopped.await;
            })
            .await
        });
        wait_ready(&f).await;
        let response = f
            .client
            .post(format!("{}/api/v1/session", f.config.https.origin))
            .header("origin", &f.config.https.origin)
            .header("x-rx-client", "browser-v1")
            .json(&json!({"principal":"admin","password":"composition-test-password"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let context: Value = f
            .client
            .get(format!(
                "{}/api/v1/package-intake-context?cell=cell%2Fa",
                f.config.https.origin
            ))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(context["registration"].is_object());
        if iteration == 0 {
            body = Some(
                json!({"request_key":id(),"command":{"id":id(),"cell":"cell/a","title":"Signed process for review","relative_path":"test-package","object":object,"configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"]}}),
            );
        }
        let response = f
            .client
            .post(format!("{}/api/v1/package-intakes", f.config.https.origin))
            .header("origin", &f.config.https.origin)
            .header("x-rx-client", "browser-v1")
            .header("cookie", &cookie)
            .json(body.as_ref().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let receipt: Value = response.json().await.unwrap();
        assert_eq!(receipt["state"], "AWAITING_REVIEW");
        assert_eq!(receipt["terminal"], "panel/a");
        if let Some(old) = &original {
            assert_eq!(old, &receipt);
        } else {
            original = Some(receipt);
        }
        let page: Value = f
            .client
            .get(format!(
                "{}/api/v1/package-intakes?cell=cell%2Fa",
                f.config.https.origin
            ))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(page["packages"].as_array().unwrap().len(), 1);
        assert_eq!(page["packages"][0]["activation_authorized"], false);
        assert_eq!(
            page["packages"][0]["review_context_current"],
            iteration == 0
        );
        if iteration == 0 {
            let policy_path = &f.config.package_intake.as_ref().unwrap().policy.path;
            fs::write(policy_path, b"changed after startup").unwrap();
            let mut fresh = body.as_ref().unwrap().clone();
            fresh["request_key"] = json!(id());
            fresh["command"]["id"] = json!(id());
            let failed = f
                .client
                .post(format!("{}/api/v1/package-intakes", f.config.https.origin))
                .header("origin", &f.config.https.origin)
                .header("x-rx-client", "browser-v1")
                .header("cookie", &cookie)
                .json(&fresh)
                .send()
                .await
                .unwrap();
            assert_eq!(failed.status(), reqwest::StatusCode::CONFLICT);
            fs::write(policy_path, &p.policy_bytes).unwrap();
        }
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
    let mut tampered = f.config.clone();
    tampered.package_intake.as_mut().unwrap().policy.sha256 = Digest::from_bytes([99; 32]);
    fs::write(&f.path, canonical::bytes(&tampered).unwrap()).unwrap();
    assert!(
        serve(&f.path, TestClock, std::future::ready(()))
            .await
            .is_err()
    );
}

#[path = "../../rx-application/tests/support/process_review.rs"]
mod review_support;
#[tokio::test]
async fn configured_process_review_records_separate_account_approval_over_terminal_https() {
    let mut f = fixture();
    let mut catalog: Catalog =
        canonical::decode_json(&f.config.catalog.read(false).unwrap()).unwrap();
    let cell = catalog.cells[0].clone();
    catalog.principals.push(Principal {
        id: name("reviewer"),
        client_namespace: name("reviewer"),
        roles: [Role::Verifier, Role::ReleaseManager].into_iter().collect(),
        cells: [cell.id.clone()].into_iter().collect(),
        active: true,
    });
    f.config.catalog = pin(
        f._dir.path(),
        "catalog.json",
        &canonical::bytes(&catalog).unwrap(),
    );
    let mut credentials: rx_api::auth::Credentials =
        canonical::decode_json(&f.config.credentials.read(true).unwrap()).unwrap();
    credentials.accounts.push(rx_api::auth::LocalAccount {
        principal: name("reviewer"),
        password_hash: credentials.accounts[0].password_hash.clone(),
    });
    f.config.credentials = pin(
        f._dir.path(),
        "credentials.json",
        &canonical::bytes(&credentials).unwrap(),
    );
    let p = review_support::fixture(&cell, Digest::from_bytes([71; 32]));
    f.config.package_intake = Some(PackageIntake {
        device_review_authority: None,
        qualification_policy: None,
        import_root: p._dir.path().to_path_buf(),
        policy: pin(f._dir.path(), "package-policy.json", &p.policy_bytes),
        review_authority: Some(pin(
            f._dir.path(),
            "review-authority.json",
            &canonical::bytes(&p.authority).unwrap(),
        )),
    });
    assert!(p.package.is_dir() && p.policy_path.is_file() && p.policy.fingerprint().is_ok());
    p.store.verify(&p.object, &p.policy).unwrap();
    fs::write(&f.path, canonical::bytes(&f.config).unwrap()).unwrap();
    initialize(&f.path).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let path = f.path.clone();
    let task = tokio::spawn(async move {
        serve(&path, TestClock, async {
            let _ = stopped.await;
        })
        .await
    });
    wait_ready(&f).await;
    let mut cookies = Vec::new();
    for principal in ["admin", "reviewer"] {
        let response = f
            .client
            .post(format!("{}/api/v1/session", f.config.https.origin))
            .header("origin", &f.config.https.origin)
            .header("x-rx-client", "browser-v1")
            .json(&json!({"principal":principal,"password":"composition-test-password"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        cookies.push(
            response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned(),
        );
    }
    let get = |path: &str, cookie: &str| {
        f.client
            .get(format!("{}{path}", f.config.https.origin))
            .header("cookie", cookie)
    };
    let post = |path: &str, cookie: &str| {
        f.client
            .post(format!("{}{path}", f.config.https.origin))
            .header("cookie", cookie)
            .header("origin", &f.config.https.origin)
            .header("x-rx-client", "browser-v1")
    };
    let context: Value = get("/api/v1/package-intake-context?cell=cell%2Fa", &cookies[0])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intake = id();
    let receipt=post("/api/v1/package-intakes",&cookies[0]).json(&json!({"request_key":id(),"command":{"id":intake,"cell":"cell/a","title":"TLS review fixture","relative_path":"package","object":p.object,"configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"]}})).send().await.unwrap();
    assert_eq!(receipt.status(), reqwest::StatusCode::OK);
    let review = id();
    let response=post("/api/v1/process-reviews",&cookies[0]).json(&json!({"request_key":id(),"command":{"id":review,"intake":intake,"cell":"cell/a","configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"],"binding_selections":{"load":cell.steps[0].id}}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let job: rx_application::process_review::Job = response.json().await.unwrap();
    let (report, signature, resolved) = p.report(&job);
    let dir = p._dir.path().join("report");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("verification.json"),
        canonical::bytes(&report).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("verification.sig.json"),
        canonical::bytes(&signature).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("resolved.json"), resolved).unwrap();
    let response=post("/api/v1/process-review/reports",&cookies[0]).json(&json!({"request_key":id(),"command":{"review":review,"cell":"cell/a","expected":null,"directory":"report","report_digest":report.digest().unwrap()}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let version: rx_application::process_review::Version = response.json().await.unwrap();
    assert!(version.ready_for_software_approval);
    let response=post("/api/v1/process-review/decisions",&cookies[1]).json(&json!({"request_key":id(),"command":{"review":review,"cell":"cell/a","report_revision":version.revision,"review_digest":version.review_digest,"expected":null,"choice":"APPROVE","note":"Checked the immutable software review materials"}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let decision: Value = response.json().await.unwrap();
    assert_eq!(decision["decided_by"], "reviewer");
    let view: Value = get(
        &format!("/api/v1/process-review?cell=cell%2Fa&id={review}"),
        &cookies[1],
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(view["approval_matches_current_review"], true);
    assert_eq!(view["activation_authorized"], false);
    let change_id = id();
    let response=post("/api/v1/process-changes",&cookies[0]).json(&json!({"request_key":id(),"command":{"id":change_id,"cell":"cell/a","review":{"id":review,"revision":version.revision,"review_digest":version.review_digest,"decision_revision":decision["revision"]},"reason":"Prepare the reviewed process with no active devices"}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let proposed: Value = response.json().await.unwrap();
    let target = |c: &Value| json!({"change":change_id,"cell":"cell/a","expected":c["revision"],"plan_digest":c["plan_digest"]});
    let response=post("/api/v1/process-change/impact-review",&cookies[1]).json(&json!({"request_key":id(),"command":{"target":target(&proposed),"note":"Whole cell requalification and Host confirmation required"}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let impact: Value = response.json().await.unwrap();
    let response = post("/api/v1/process-change/stage", &cookies[1])
        .json(&json!({"request_key":id(),"command":target(&impact)}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let staged: Value = response.json().await.unwrap();
    let response = post("/api/v1/process-change/prepare", &cookies[1])
        .json(&json!({"request_key":id(),"command":{"target":target(&staged),"refresh":false}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let prepared: Value = response.json().await.unwrap();
    assert_eq!(prepared["state"], "STAGED");
    assert!(prepared["preparation"].is_object());
    let value: Value = get(
        &format!("/api/v1/process-change?cell=cell%2Fa&id={change_id}"),
        &cookies[1],
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(value["applied"], false);
    assert_eq!(value["activation_authorized"], false);
    assert!(
        value["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["kind"] == "HOST_CONFIGURATION_ACKNOWLEDGEMENT_REQUIRED")
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
