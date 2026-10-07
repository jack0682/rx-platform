//! Actual mTLS reporter ingress with the real application writer. No device authority.
use rx_api::grpc::{Configuration, PlatformIngress, TlsMaterial};
use rx_application::*;
use rx_domain::{canonical, component::*, resident_reporting::*, types::*};
use rx_protocol::{base, resident_reporting as wire};
use rx_runtime::{
    application::{Application, ApplicationPort, CallFuture, Command, Handle, Reply},
    writer::{Status, Writer, WriterError},
};
use rx_storage::SqliteRepository;
use sha2::Digest as _;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity as TlsIdentity};

fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn binding() -> Vec<u8> {
    sha2::Sha256::digest(include_bytes!(
        "../../../spec/resident-reporting/v1/binding.json"
    ))
    .to_vec()
}
struct Clock;
impl rx_application::Clock for Clock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "reporter-test-clock".into(),
            ticks_ns: Counter(1000),
        }
    }
}
struct Deny;
impl QualificationAuthority for Deny {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type App = Handle<Application<SqliteRepository, Clock, Deny>>;
struct LostAck {
    handle: App,
    lose: AtomicBool,
    lose_acceptance: AtomicBool,
    stall: Arc<AtomicBool>,
}
impl ApplicationPort for LostAck {
    fn request(&self, command: Command) -> CallFuture<'_> {
        let acceptance = matches!(command, Command::RegistrationTargetAcceptance { .. });
        let lose = matches!(command, Command::PublishResidentReport { .. })
            && self.lose.swap(false, Ordering::SeqCst);
        let stalled = matches!(
            command,
            Command::ResidentReportHead { .. } | Command::PublishResidentReport { .. }
        ) && self.stall.load(Ordering::SeqCst);
        Box::pin(async move {
            if stalled {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            let reply = self.handle.call(command).await?;
            if lose || (acceptance && self.lose_acceptance.swap(false, Ordering::SeqCst)) {
                Err(WriterError::Unavailable)
            } else {
                Ok(reply)
            }
        })
    }
    fn status(&self) -> Status {
        self.handle.status()
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    handle: App,
    admin: rx_application::Identity,
    installation: Installation,
    endpoint: String,
    ca: String,
    keys: BTreeMap<String, (String, String)>,
    release: Digest,
    stall: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
impl Fixture {
    async fn channel(&self, who: &str) -> Channel {
        let (cert, key) = &self.keys[who];
        tonic::transport::Endpoint::from_shared(self.endpoint.clone())
            .unwrap()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(3))
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name("localhost")
                    .ca_certificate(Certificate::from_pem(&self.ca))
                    .identity(TlsIdentity::from_pem(cert, key)),
            )
            .unwrap()
            .connect()
            .await
            .unwrap()
    }
    fn hello(&self, who: &str) -> wire::OpenReporter {
        wire::OpenReporter {
            peer_id: who.into(),
            peer_boot: id().to_string(),
            installation_id: self.installation.id.to_string(),
            store_generation: self.installation.store_generation.to_string(),
            shared_clock_id: self.installation.clock_id.clone(),
            release_digest: self.release.as_bytes().to_vec(),
            binding_hash: binding(),
        }
    }
    async fn issue(
        &self,
        peer: &Peer,
        registration: &VersionedRegistration,
    ) -> rx_application::resident_reporting::ScopeView {
        let Reply::Component(component) = self
            .handle
            .call(Command::CreateComponent {
                identity: self.admin.clone(),
                key: id(),
                input: rx_application::resident_component::Create {
                    declaration: registration.registration.declaration.clone(),
                },
            })
            .await
            .unwrap()
        else {
            panic!("component");
        };
        let Reply::ResidentReportingScopeView(scope) = self
            .handle
            .call(Command::IssueResidentReporting {
                identity: self.admin.clone(),
                key: id(),
                input: rx_application::resident_reporting::Issue {
                    component: component.record.registration.id,
                    expected_component_revision: component.revision,
                    reporter_session: peer.id.clone(),
                    source_registration: registration.registration.id.clone(),
                    source_revision: registration.revision,
                },
            })
            .await
            .unwrap()
        else {
            panic!("scope");
        };
        *scope
    }
}
async fn fixture() -> Fixture {
    use rcgen::*;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("platform.db");
    let session = id();
    let admin_session = session.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(db)?,
            Clock,
            Deny,
            id(),
            Principal {
                id: name("admin"),
                client_namespace: name("admin"),
                roles: [Role::AccountAdmin, Role::Engineer].into(),
                cells: Default::default(),
                active: true,
            },
        )?;
        engine.authenticated_session(
            &name("admin"),
            session.clone(),
            TimePoint {
                clock_id: "reporter-test-clock".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let admin = rx_application::Identity {
            principal: name("admin"),
            session,
            terminal: None,
        };
        for (label, role) in [
            ("reporter", Role::Observer),
            ("other", Role::Observer),
            ("host", Role::Host),
        ] {
            engine.put_principal(
                &admin,
                Principal {
                    id: name(label),
                    client_namespace: name(label),
                    roles: [role].into(),
                    cells: Default::default(),
                    active: true,
                },
                None,
            )?;
        }
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let handle = Handle::new(writer);
    let Reply::Installation(installation) = handle.call(Command::Installation).await.unwrap()
    else {
        panic!("installation");
    };
    let mut parameters = CertificateParams::new(vec![]).unwrap();
    parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    parameters.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(parameters, KeyPair::generate().unwrap()).unwrap();
    let leaf = |label: &str, usage: ExtendedKeyUsagePurpose| {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec![label.into()]).unwrap();
        p.extended_key_usages = vec![usage];
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (p.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let mut allowed = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for who in ["reporter", "other", "host", "unregistered"] {
        let (cert, key) = leaf(who, ExtendedKeyUsagePurpose::ClientAuth);
        if who != "unregistered" {
            allowed.insert(
                Digest::from_bytes(sha2::Sha256::digest(cert.der().as_ref()).into()),
                name(who),
            );
        }
        keys.insert(who.to_string(), (cert.pem(), key.serialize_pem()));
    }
    let release = Digest::from_bytes([55; 32]);
    let stall = Arc::new(AtomicBool::new(false));
    let ingress = PlatformIngress::new(
        Arc::new(LostAck {
            handle: handle.clone(),
            lose: AtomicBool::new(true),
            lose_acceptance: AtomicBool::new(true),
            stall: stall.clone(),
        }),
        Configuration {
            installation: installation.clone(),
            release_digest: release,
            allowed_certificates: allowed,
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let (stop, rx) = tokio::sync::oneshot::channel();
    let tls = TlsMaterial {
        server_certificate_pem: server.pem().into_bytes(),
        server_key_pem: server_key.serialize_pem().into_bytes(),
        client_ca_pem: ca.pem().into_bytes(),
    };
    let task = tokio::spawn(ingress.serve(listener, tls, async {
        let _ = rx.await;
    }));
    Fixture {
        dir,
        handle,
        admin: rx_application::Identity {
            principal: name("admin"),
            session: admin_session,
            terminal: None,
        },
        installation,
        endpoint,
        ca: ca.pem(),
        keys,
        release,
        stall,
        stop: Some(stop),
        task: Some(task),
    }
}
fn decode<T: serde::de::DeserializeOwned>(p: wire::Payload) -> T {
    assert_eq!(p.sha256, sha2::Sha256::digest(&p.data).as_slice());
    canonical::decode_json(&p.data).unwrap()
}
fn publish(peer: &Peer, key: &Id, report: &Report) -> wire::PublishReport {
    let data = canonical::bytes(report).unwrap();
    wire::PublishReport {
        session_id: peer.id.to_string(),
        request_key: key.to_string(),
        payload_sha256: sha2::Sha256::digest(&data).to_vec(),
        payload: data,
        binding_hash: binding(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mtls_reporter_is_scoped_and_cannot_become_a_host_or_author() {
    use wire::resident_reporting_service_client::ResidentReportingServiceClient as Client;
    let f = fixture().await;
    for who in ["unregistered", "host"] {
        let mut client = Client::new(f.channel(who).await);
        assert_eq!(
            client.open(f.hello(who)).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
    }
    let channel = f.channel("reporter").await;
    let mut client = Client::new(channel.clone());
    let mut wrong = f.hello("reporter");
    wrong.binding_hash = vec![0; 32];
    assert_eq!(
        client.open(wrong).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let mut wrong = f.hello("other");
    wrong.installation_id = id().to_string();
    assert_eq!(
        client.open(wrong).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let peer: Peer = decode(client.open(f.hello("reporter")).await.unwrap().into_inner());
    let mut base_client = base::session_service_client::SessionServiceClient::new(channel);
    let base_manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../spec/contracts/v1.0/protocol_manifest.json"
    ))
    .unwrap();
    let hash = sha2::Sha256::digest(canonical::bytes(&base_manifest).unwrap()).to_vec();
    let host_hello = base::PeerHello {
        peer_id: "reporter".into(),
        role: base::Role::Host as i32,
        boot_id: id().to_string(),
        installation_id: f.installation.id.to_string(),
        store_generation: f.installation.store_generation.to_string(),
        supported_versions: vec![base::Version {
            major: 1,
            minor: 0,
            schema_hash: hash,
        }],
        release_digest: f.release.as_bytes().to_vec(),
        journal_id: Some(id().to_string()),
        last_seq: Some(0),
        shared_clock_id: f.installation.clock_id.clone(),
    };
    assert_eq!(
        base_client.open(host_hello).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    assert!(
        f.handle
            .call(Command::OpenEvidenceProducer {
                principal: name("reporter"),
                peer_boot: id(),
                journal: id(),
                authentication_binding: Digest::from_bytes([7; 32])
            })
            .await
            .is_err()
    );
    let registration = VersionedRegistration {
        revision: Counter(1),
        registration: Registration {
            id: id(),
            declaration: Declaration {
                label: name("status"),
                catalog: CatalogReference {
                    program: name("test/status"),
                    digest: Digest::from_bytes([8; 32]),
                },
            },
            state: RegistrationState::Accepted,
        },
    };
    let scope = f.issue(&peer, &registration).await;
    let mut other = Client::new(f.channel("other").await);
    let other_peer: Peer = decode(other.open(f.hello("other")).await.unwrap().into_inner());
    assert_eq!(
        other
            .inspect(wire::InspectScope {
                session_id: other_peer.id.to_string(),
                scope_id: scope.scope.id.to_string(),
                binding_hash: binding()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let report = Report {
        scope: scope.scope.id.clone(),
        source: Binding {
            registration: scope.scope.source_registration.clone(),
            registration_revision: scope.scope.source_revision,
            catalog: scope.scope.catalog.clone(),
            run: id(),
            selection: name("status"),
            instance: id(),
        },
        sequence: Counter(1),
        state: ExecutionState::Running,
        pid: Some(4242),
        exit_code: None,
        detail: "attributed test observation".into(),
    };
    let key = id();
    let request = publish(&peer, &key, &report);
    let mut bad = request.clone();
    bad.payload_sha256 = vec![0; 32];
    assert_eq!(
        client.publish(bad).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut extra = serde_json::to_value(&report).unwrap();
    extra["work_use_permission"] = serde_json::json!("ALLOWED");
    let mut noncanonical = request.payload.clone();
    noncanonical.push(b' ');
    for data in [canonical::bytes(&extra).unwrap(), noncanonical] {
        let mut bad = request.clone();
        bad.payload_sha256 = sha2::Sha256::digest(&data).to_vec();
        bad.payload = data;
        assert_eq!(
            client.publish(bad).await.unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }
    let mut oversized = request.clone();
    oversized.payload = vec![b' '; 65537];
    oversized.payload_sha256 = sha2::Sha256::digest(&oversized.payload).to_vec();
    assert_eq!(
        client.publish(oversized).await.unwrap_err().code(),
        tonic::Code::ResourceExhausted
    );
    assert_eq!(
        client.publish(request.clone()).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    let receipt: Receipt = decode(client.publish(request.clone()).await.unwrap().into_inner());
    let repeated: Receipt = decode(client.publish(request).await.unwrap().into_inner());
    assert_eq!(receipt, repeated);
    assert_eq!(receipt.work_use_permission, WorkUse::NotEvaluated);
    f.handle
        .call(Command::RevokeResidentReporting {
            identity: f.admin.clone(),
            key: id(),
            input: rx_application::resident_reporting::Revoke {
                scope: scope.scope.id,
                expected_revision: scope.revision,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        client
            .publish(publish(&peer, &key, &report))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "tools/test_resident_reporting.py supplies the separately built real Supervisor fixture"]
async fn actual_supervisor_child_reports_running_and_owned_exit_through_the_scoped_client() {
    run_resident_scene(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "tools/test_resident_reporting.py supplies the separately built real Supervisor fixture"]
async fn actual_reporter_outage_preserves_local_stop_and_owner_approved_restart() {
    run_resident_scene(true).await;
}

async fn run_resident_scene(outage: bool) {
    let executable =
        std::env::var("RX_RESIDENT_REPORT_FIXTURE").expect("separate S fixture required");
    let f = fixture().await;
    let output = f.dir.path().join("reporter");
    std::fs::create_dir(&output).unwrap();
    for (name, bytes) in [
        ("ca.pem", f.ca.as_bytes()),
        ("client.pem", f.keys["reporter"].0.as_bytes()),
        ("client.key", f.keys["reporter"].1.as_bytes()),
    ] {
        std::fs::write(f.dir.path().join(name), bytes).unwrap();
    }
    let config = f.dir.path().join("reporter.json");
    std::fs::write(&config,serde_json::to_vec(&serde_json::json!({"endpoint":f.endpoint,"ca":f.dir.path().join("ca.pem"),"certificate":f.dir.path().join("client.pem"),
        "key":f.dir.path().join("client.key"),"output":output,"installation":f.installation.id,"store_generation":f.installation.store_generation,
        "clock":f.installation.clock_id,"release":f.release,"outage":outage})).unwrap()).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        std::process::Command::new(executable)
            .arg(config)
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(70);
    let ready = output.join("ready.json");
    while !ready.is_file() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "S fixture exited before ready"
        );
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
    let peer: Peer = serde_json::from_value(value["peer"].clone()).unwrap();
    // VersionedRegistration is intentionally a read model; decode its value fields separately.
    let registration = VersionedRegistration {
        revision: serde_json::from_value(value["registration"]["revision"].clone()).unwrap(),
        registration: serde_json::from_value(value["registration"]["registration"].clone())
            .unwrap(),
    };
    let scope = f.issue(&peer, &registration).await;
    std::fs::write(
        output.join("authorization.pending"),
        serde_json::to_vec(
            &serde_json::json!({"scope":scope.scope.id,"component":scope.scope.component}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::rename(
        output.join("authorization.pending"),
        output.join("authorization.json"),
    )
    .unwrap();
    let mut outage_started = false;
    let mut continued = false;
    loop {
        if outage && !outage_started && output.join("running.json").is_file() {
            f.stall.store(true, Ordering::SeqCst);
            std::fs::write(output.join("stop-approved.json"), b"{}").unwrap();
            outage_started = true;
        }
        if outage && !continued && output.join("restart-ready.json").is_file() {
            let restarted: Peer =
                serde_json::from_slice(&std::fs::read(output.join("restart-ready.json")).unwrap())
                    .unwrap();
            f.stall.store(false, Ordering::SeqCst);
            let Reply::ResidentReportingScopeView(successor) = f
                .handle
                .call(Command::ContinueResidentReporting {
                    identity: f.admin.clone(),
                    key: id(),
                    input: rx_application::resident_reporting::Continue {
                        scope: scope.scope.id.clone(),
                        expected_revision: scope.revision,
                        reporter_session: restarted.id,
                    },
                })
                .await
                .unwrap()
            else {
                panic!("continuation");
            };
            let bytes = serde_json::to_vec(&serde_json::json!({"scope":successor.scope.id,"component":successor.scope.component})).unwrap();
            std::fs::write(output.join("authorization-restart.pending"), bytes).unwrap();
            std::fs::rename(
                output.join("authorization-restart.pending"),
                output.join("authorization-restart.json"),
            )
            .unwrap();
            continued = true;
        }
        if let Some(exit) = child.0.try_wait().unwrap() {
            assert!(exit.success(), "S fixture exit: {exit}");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["status"], "PASS");
    if outage {
        assert!(outage_started && continued);
        assert!(result["local_stop_ms"].as_u64().unwrap() < 2000);
        assert_eq!(result["retained_during_outage"], true);
    }
    let stopped: Receipt = serde_json::from_value(result["stopped"].clone()).unwrap();
    let Reply::ResidentReportView(view) = f
        .handle
        .call(Command::GetResidentReport {
            identity: f.admin.clone(),
            component: scope.scope.component,
            instance: stopped.report.source.instance.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("report view");
    };
    assert_eq!(view.receipt, stopped);
    assert_eq!(view.receipt.report.state, ExecutionState::Exited);
    assert_eq!(view.receipt.report.exit_code, Some(0));
    assert_eq!(
        view.receipt.report.source.registration,
        registration.registration.id
    );
    assert_eq!(
        view.receipt.execution_ownership,
        Ownership::NotEstablishedByReport
    );
    assert_eq!(view.receipt.work_use_permission, WorkUse::NotEvaluated);
    if let Ok(path) = std::env::var("RX_RESIDENT_REPORT_EVIDENCE") {
        let path = if outage {
            format!("{path}.outage.json")
        } else {
            path
        };
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "tools/test_registration_intake.py supplies the actual source fixture and prior reader"]
async fn actual_registration_source_is_imported_through_authenticated_http_and_original_request_recovery()
 {
    use axum::{
        body::{Body, to_bytes},
        extract::ConnectInfo,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = fixture().await;
    let source = f.dir.path().join("source.db");
    let seed = std::env::var("RX_REGISTRATION_SOURCE").unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let input = std::path::PathBuf::from(format!("{seed}{suffix}"));
        if input.is_file() {
            std::fs::copy(input, format!("{}{suffix}", source.display())).unwrap();
        }
    }
    let freeze = id();
    let output =
        std::process::Command::new(std::env::var("RX_REGISTRATION_SOURCE_FIXTURE").unwrap())
            .args([
                source.to_str().unwrap(),
                f.installation.id.as_str(),
                freeze.as_str(),
            ])
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    struct LostAcceptance {
        inner: App,
        lose: AtomicBool,
    }
    impl ApplicationPort for LostAcceptance {
        fn request(&self, command: Command) -> CallFuture<'_> {
            let lose = matches!(command, Command::FinishComponentIntake(_))
                && self.lose.swap(false, Ordering::SeqCst);
            Box::pin(async move {
                let reply = self.inner.call(command).await?;
                if lose {
                    Err(WriterError::Unavailable)
                } else {
                    Ok(reply)
                }
            })
        }
        fn status(&self) -> Status {
            self.inner.status()
        }
    }
    let service = rx_runtime::component_intake::Service::configure(
        Arc::new(LostAcceptance {
            inner: f.handle.clone(),
            lose: AtomicBool::new(true),
        }),
        [(
            name("source"),
            rx_runtime::component_intake::Source {
                path: source.clone(),
                owner: name("admin"),
            },
        )]
        .into(),
    )
    .await
    .unwrap();
    let credentials = rx_api::auth::Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: vec![rx_api::auth::LocalAccount {
            principal: name("admin"),
            password_hash: rx_api::auth::password_hash("fixture-only").unwrap(),
        }],
    };
    let app = rx_api::router(
        service,
        credentials,
        rx_api::LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
    )
    .unwrap();
    fn request(
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8080")
            .header("origin", "http://127.0.0.1:8080")
            .header("content-type", "application/json")
            .header("x-rx-client", "browser-v1");
        if let Some(cookie) = cookie {
            r = r.header("cookie", cookie);
        }
        let mut r = r.body(Body::from(body.to_string())).unwrap();
        r.extensions_mut()
            .insert(ConnectInfo::<std::net::SocketAddr>(
                "127.0.0.1:12345".parse().unwrap(),
            ));
        r
    }
    async fn body(response: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let data = to_bytes(response.into_body(), 32 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&data).unwrap())
    }
    let login = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/session",
            None,
            serde_json::json!({"principal":"admin","password":"fixture-only"}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(login.status(), StatusCode::OK);
    let (_, context) = body(
        app.clone()
            .oneshot(request(
                "GET",
                "/api/v1/registration-source?source=source",
                Some(&cookie),
                serde_json::Value::Null,
            ))
            .await
            .unwrap(),
    )
    .await;
    let (_, overview) = body(
        app.clone()
            .oneshot(request(
                "GET",
                "/api/v1/overview",
                Some(&cookie),
                serde_json::Value::Null,
            ))
            .await
            .unwrap(),
    )
    .await;
    let cell_count = overview["cells"].as_array().unwrap().len();
    assert_eq!(cell_count, 0);
    let payload = serde_json::json!({"request_key":id(),"command":{"source":"source","freeze":freeze,"expected_binding":context["binding"]}});
    let before = std::fs::read(&source).unwrap();
    assert_eq!(
        body(
            app.clone()
                .oneshot(request(
                    "POST",
                    "/api/v1/registration-transfers",
                    None,
                    payload.clone()
                ))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut forged = payload.clone();
    forged["command"]["sealed"] = serde_json::json!(true);
    assert_eq!(
        body(
            app.clone()
                .oneshot(request(
                    "POST",
                    "/api/v1/registration-transfers",
                    Some(&cookie),
                    forged
                ))
                .await
                .unwrap()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
    // Each rejected source is private to this test. Refusal must not create/upgrade it.
    let held = f.dir.path().join("source.held");
    std::fs::rename(&source, &held).unwrap();
    let unavailable = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/registration-transfers",
            Some(&cookie),
            payload.clone(),
        ))
        .await
        .unwrap();
    assert!(!unavailable.status().is_success());
    assert!(!source.exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&held, &source).unwrap();
        let alias = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/v1/registration-transfers",
                Some(&cookie),
                payload.clone(),
            ))
            .await
            .unwrap();
        assert!(!alias.status().is_success());
        std::fs::remove_file(&source).unwrap();
        std::fs::hard_link(&held, &source).unwrap();
        let alias = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/v1/registration-transfers",
                Some(&cookie),
                payload.clone(),
            ))
            .await
            .unwrap();
        assert!(!alias.status().is_success());
        std::fs::remove_file(&source).unwrap();
    }
    SqliteRepository::open(&source).unwrap().close().unwrap();
    let unsealed = std::fs::read(&source).unwrap();
    let refused = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/registration-transfers",
            Some(&cookie),
            payload.clone(),
        ))
        .await
        .unwrap();
    assert!(!refused.status().is_success());
    assert_eq!(std::fs::read(&source).unwrap(), unsealed);
    std::fs::remove_file(&source).unwrap();
    std::fs::rename(&held, &source).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let (status, lost) = body(
        app.clone()
            .oneshot(request(
                "POST",
                "/api/v1/registration-transfers",
                Some(&cookie),
                payload.clone(),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{lost}");
    assert_eq!(lost["outcome_unknown"], true);
    // Recovery must not require another source read after target acceptance committed.
    std::fs::rename(&source, f.dir.path().join("source.offline")).unwrap();
    let (status, receipt) = body(
        app.clone()
            .oneshot(request(
                "POST",
                "/api/v1/registration-transfers",
                Some(&cookie),
                payload,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["work_use_permission"], "NOT_EVALUATED");
    for declaration in original["freeze"]["declarations"].as_array().unwrap() {
        let cid = declaration["document"]["value"]["id"].as_str().unwrap();
        let (status, current) = body(
            app.clone()
                .oneshot(request(
                    "GET",
                    &format!("/api/v1/component?id={cid}"),
                    Some(&cookie),
                    serde_json::Value::Null,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(current["revision"], declaration["revision"]);
        assert_eq!(
            current["record"]["registration"],
            declaration["document"]["value"]
        );
    }
    let mut archived = Vec::new();
    let mut after = "0".to_string();
    loop {
        let (status, page) = body(
            app.clone()
                .oneshot(request(
                    "GET",
                    &format!("/api/v1/registration-transfer/history?id={freeze}&after={after}"),
                    Some(&cookie),
                    serde_json::Value::Null,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        archived.extend(page["events"].as_array().unwrap().clone());
        match page["next_after"].as_str() {
            Some(next) => after = next.to_string(),
            None => break,
        }
    }
    assert_eq!(
        serde_json::Value::Array(archived.clone()),
        original["history"]
    );
    std::fs::rename(f.dir.path().join("source.offline"), &source).unwrap();
    let reconciled = reconcile_source_over_mtls(&f, &source, &original).await;
    f.handle.close();
    f.handle.closed().await;
    let old = std::process::Command::new(std::env::var("RX_REGISTRATION_PRIOR_READER").unwrap())
        .arg("inspect")
        .arg(f.dir.path().join("platform.db"))
        .output()
        .unwrap();
    assert!(!old.status.success());
    assert!(
        String::from_utf8_lossy(&old.stderr).contains("newer store schema: downgrade refused"),
        "{}",
        String::from_utf8_lossy(&old.stderr)
    );
    if let Ok(path) = std::env::var("RX_REGISTRATION_INTAKE_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"status":"PASS_TARGET_INTAKE","receipt":receipt,"original_freeze":original["freeze"],"history_records":archived.len(),"old_reader_refused":true,"source_offline_recovery":true,"cell_count":cell_count,"process_ownership":"NOT_TRANSFERRED","physical_execution":"NOT_PERFORMED","reconciliation":reconciled})).unwrap()).unwrap();
    }
}

async fn reconcile_source_over_mtls(
    f: &Fixture,
    source: &std::path::Path,
    original: &serde_json::Value,
) -> serde_json::Value {
    use std::io::{BufRead, Read, Write};
    use std::process::{Command as Process, Stdio};
    use wire::resident_reporting_service_client::ResidentReportingServiceClient;
    let executable = std::env::var("RX_REGISTRATION_RECONCILER").unwrap();
    let config = f.dir.path().join("acceptance-connection.json");
    let ca = f.dir.path().join("acceptance-ca.pem");
    let cert = f.dir.path().join("acceptance-cert.pem");
    let key = f.dir.path().join("acceptance-key.pem");
    std::fs::write(&ca, &f.ca).unwrap();
    std::fs::write(&cert, &f.keys["reporter"].0).unwrap();
    std::fs::write(&key, &f.keys["reporter"].1).unwrap();
    std::fs::write(&config,serde_json::to_vec(&serde_json::json!({
        "schema":"rx.resident-report-connection.v1","endpoint":f.endpoint,"server_name":"localhost",
        "ca":ca,"certificate":cert,"private_key":key,"principal":"reporter",
        "installation":f.installation.id,"store_generation":f.installation.store_generation,
        "shared_clock_id":f.installation.clock_id,"release_digest":f.release,
    })).unwrap()).unwrap();
    let declaration = &original["freeze"]["declarations"][0];
    let component: Id =
        serde_json::from_value(declaration["document"]["value"]["id"].clone()).unwrap();
    let revision: Counter = serde_json::from_value(declaration["revision"].clone()).unwrap();
    let freeze: Id =
        serde_json::from_value(original["freeze"]["record"]["request"]["id"].clone()).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    async fn line(
        mut reader: std::io::BufReader<std::process::ChildStdout>,
    ) -> (
        std::io::BufReader<std::process::ChildStdout>,
        serde_json::Value,
    ) {
        tokio::time::timeout(
            Duration::from_secs(10),
            tokio::task::spawn_blocking(move || {
                let mut text = String::new();
                reader.read_line(&mut text).unwrap();
                let value = serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("CLI output {text:?}: {e}"));
                (reader, value)
            }),
        )
        .await
        .unwrap()
        .unwrap()
    }
    let mut results = Vec::new();
    let mut old_query = None;
    for attempt in 0..2 {
        let mut child = Child(
            Process::new(&executable)
                .arg("reconcile")
                .arg(source)
                .arg(&config)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = std::io::BufReader::new(child.0.stdout.take().unwrap());
        let (stdout, ready) = line(stdout).await;
        assert_eq!(ready["state"], "AWAITING_OWNER_SCOPE");
        let peer: Peer = serde_json::from_value(ready["peer"].clone()).unwrap();
        assert_eq!(ready["source"], original["freeze"]["record"]);
        let mut transport = ResidentReportingServiceClient::new(f.channel("reporter").await);
        if let Some(old) = old_query.take() {
            assert!(transport.acceptance(old).await.is_err());
        }
        let query = wire::ReadAcceptance {
            session_id: peer.id.to_string(),
            scope_id: id().to_string(),
            freeze_id: freeze.to_string(),
            binding_hash: binding(),
        };
        assert!(transport.acceptance(query.clone()).await.is_err());
        let Reply::ResidentReportingScopeView(scope) = f
            .handle
            .call(Command::IssueResidentReporting {
                identity: f.admin.clone(),
                key: id(),
                input: rx_application::resident_reporting::Issue {
                    component: component.clone(),
                    expected_component_revision: revision,
                    reporter_session: peer.id,
                    source_registration: component.clone(),
                    source_revision: revision,
                },
            })
            .await
            .unwrap()
        else {
            panic!("canonical scope")
        };
        let query = wire::ReadAcceptance {
            scope_id: scope.scope.id.to_string(),
            ..query
        };
        let mut other = ResidentReportingServiceClient::new(f.channel("other").await);
        assert!(other.acceptance(query.clone()).await.is_err());
        let wrong = wire::ReadAcceptance {
            freeze_id: id().to_string(),
            ..query.clone()
        };
        assert!(transport.acceptance(wrong).await.is_err());
        let wrong = wire::ReadAcceptance {
            binding_hash: vec![0; 32],
            ..query.clone()
        };
        assert!(transport.acceptance(wrong).await.is_err());
        if attempt == 0 {
            let lost = transport.acceptance(query.clone()).await.unwrap_err();
            assert_eq!(lost.code(), tonic::Code::Unavailable);
        }
        old_query = Some(query.clone());
        writeln!(
            child.0.stdin.as_mut().unwrap(),
            "{}",
            serde_json::json!({"component":component,"scope":scope.scope.id})
        )
        .unwrap();
        child.0.stdin.take();
        let (_stdout, result) = line(stdout).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(std::time::Instant::now() < deadline, "CLI did not finish");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let mut stderr = String::new();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(status.success(), "{stderr}");
        assert_eq!(result["state"], "RECORDED_FROM_AUTHENTICATED_PLATFORM");
        assert_eq!(
            result["acceptance"]["original"],
            original["freeze"]["record"]
        );
        assert_eq!(result["process_ownership"], "NOT_TRANSFERRED");
        results.push(result);
    }
    assert_eq!(results[0]["acceptance"], results[1]["acceptance"]);
    assert_ne!(
        results[0]["current_observation"]["peer"]["peer_boot"],
        results[1]["current_observation"]["peer"]["peer_boot"]
    );
    let output = Process::new(&executable)
        .arg("inspect")
        .arg(source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let inspected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        inspected["platform_acceptance"],
        "RECORDED_FROM_AUTHENTICATED_PLATFORM"
    );
    assert_eq!(inspected["record"], original["freeze"]["record"]);
    let mut registry = SqliteRepository::open_sealed_existing(source).unwrap();
    use rx_ports::Repository;
    let events = registry.control_events_after(Counter(0), 128).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.document.schema.as_str() == "rx.registration-acceptance-recorded.v1")
            .count(),
        1
    );
    registry.close().unwrap();
    serde_json::json!({"status":"PASS_SCOPED_MTLS_RECONCILIATION","first":results[0],"after_restart":results[1],"single_local_acceptance_event":true,"lost_target_read_recovered":true})
}
