//! Actual Linux daemon, verified release content and P owner/transport path. No device operations.
#![cfg(target_os = "linux")]
use axum::{
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use rx_api::grpc::{Configuration, PlatformIngress, TlsMaterial};
use rx_application::*;
use rx_domain::{component::*, resident_execution as data, types::*};
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
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command as Process},
};
use tower::ServiceExt;
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
struct Clock {
    domain: String,
}
impl rx_application::Clock for Clock {
    fn now(&self) -> TimePoint {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        TimePoint {
            clock_id: self.domain.clone(),
            ticks_ns: Counter((t.tv_sec as u64) * 1_000_000_000 + t.tv_nsec as u64),
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
struct Faults {
    inner: App,
    lose_approval: AtomicBool,
    lose_observation: AtomicBool,
    stall: AtomicBool,
}
impl ApplicationPort for Faults {
    fn request(&self, command: Command) -> CallFuture<'_> {
        let approval = matches!(command, Command::ApproveResidentExecution { .. });
        let observation = matches!(command, Command::ObserveResidentExecution { .. });
        let stalled = matches!(
            command,
            Command::ObserveResidentExecution { .. } | Command::InspectResidentExecution { .. }
        ) && self.stall.load(Ordering::SeqCst);
        Box::pin(async move {
            if stalled {
                tokio::time::sleep(Duration::from_secs(10)).await;
                return Err(WriterError::Unavailable);
            }
            let reply = self.inner.call(command).await?;
            if (approval && self.lose_approval.swap(false, Ordering::SeqCst))
                || (observation && self.lose_observation.swap(false, Ordering::SeqCst))
            {
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
struct ChildOwner(Child);
impl Drop for ChildOwner {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}
fn request(
    method: &str,
    path: &str,
    cookie: Option<&str>,
    value: serde_json::Value,
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
    let mut r = r.body(Body::from(value.to_string())).unwrap();
    r.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    r
}
async fn body(response: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn owner(
    app: &axum::Router,
    cookie: &str,
    method: &str,
    path: &str,
    value: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    body(
        app.clone()
            .oneshot(request(method, path, Some(cookie), value))
            .await
            .unwrap(),
    )
    .await
}
async fn line(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> serde_json::Value {
    let text = tokio::time::timeout(Duration::from_secs(120), reader.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("daemon stdout ended");
    serde_json::from_str(&text).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "tools/test_resident_execution.py supplies the signed Linux runtime and built daemon"]
async fn actual_owner_preparation_grant_child_stop_and_reply_loss() {
    actual_scenario(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "tools/test_resident_execution.py supplies the signed Linux runtime and built daemon"]
async fn actual_p_outage_keeps_local_stop_and_unresolved_delivery() {
    actual_scenario(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "tools/test_resident_execution.py supplies the private Linux container and built daemon"]
async fn actual_cold_manager_loss_preserves_live_original_processes() {
    actual_scenario(false, true).await;
}
async fn actual_scenario(outage: bool, cold: bool) {
    let executable = std::env::var("RX_RESIDENT_EXECUTION_BIN").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state_name = format!("execution-{}", id());
    let config = temp.path().join("runtime.json");
    let connection = temp.path().join("connection.json");
    std::fs::write(&config,serde_json::to_vec(&serde_json::json!({"schema":"rx.resident-execution-runtime.v1","state_subdirectory":state_name})).unwrap()).unwrap();
    let output = Process::new(&executable)
        .arg("catalog")
        .arg(&config)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "catalog: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let policy: data::Enrollment = serde_json::from_value(listing["enrollment"].clone()).unwrap();
    let catalog = CatalogReference {
        program: name("rx/status-http"),
        digest: policy.programs[&name("rx/status-http")].digest,
    };
    let clock_domain = format!(
        "linux-boottime/{}",
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim()
    );
    let db = temp.path().join("platform.db");
    let session = id();
    let admin = Identity {
        principal: name("admin"),
        session: session.clone(),
        terminal: None,
    };
    let domain = clock_domain.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(db)?,
            Clock { domain },
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
                clock_id: engine.installation.clock_id.clone(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let admin = Identity {
            principal: name("admin"),
            session,
            terminal: None,
        };
        for (n, role) in [
            ("supervisor", Role::Supervisor),
            ("observer", Role::Observer),
            ("host", Role::Host),
        ] {
            engine.put_principal(
                &admin,
                Principal {
                    id: name(n),
                    client_namespace: name(n),
                    roles: [role].into(),
                    cells: Default::default(),
                    active: true,
                },
                None,
            )?;
        }
        engine.configure_resident_supervisors([(name("supervisor"), policy)].into())?;
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let handle = Handle::new(writer);
    let Reply::Installation(meta) = handle.call(Command::Installation).await.unwrap() else {
        panic!()
    };
    let faults = Arc::new(Faults {
        inner: handle.clone(),
        lose_approval: AtomicBool::new(true),
        lose_observation: AtomicBool::new(true),
        stall: AtomicBool::new(false),
    });
    use rcgen::*;
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let leaf = |n: &str, usage: ExtendedKeyUsagePurpose| {
        let k = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec![n.into()]).unwrap();
        p.extended_key_usages = vec![usage];
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (p.signed_by(&k, &ca).unwrap(), k)
    };
    let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let mut allowed = BTreeMap::new();
    let mut certs = BTreeMap::new();
    for n in ["supervisor", "observer", "host"] {
        let (cert, key) = leaf(n, ExtendedKeyUsagePurpose::ClientAuth);
        allowed.insert(
            Digest::from_bytes(sha2::Sha256::digest(cert.der().as_ref()).into()),
            name(n),
        );
        certs.insert(n, (cert.pem(), key.serialize_pem()));
    }
    let release = Digest::from_bytes([77; 32]);
    let ingress = PlatformIngress::new(
        faults.clone(),
        Configuration {
            installation: meta.clone(),
            release_digest: release,
            allowed_certificates: allowed,
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let (stop_server, done) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(ingress.serve(
        listener,
        TlsMaterial {
            server_certificate_pem: server.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            client_ca_pem: ca.pem().into_bytes(),
        },
        async {
            let _ = done.await;
        },
    ));
    let ca_file = temp.path().join("ca.pem");
    let cert_file = temp.path().join("supervisor.pem");
    let key_file = temp.path().join("supervisor.key");
    std::fs::write(&ca_file, ca.pem()).unwrap();
    std::fs::write(&cert_file, &certs["supervisor"].0).unwrap();
    std::fs::write(&key_file, &certs["supervisor"].1).unwrap();
    std::fs::write(&connection,serde_json::to_vec(&serde_json::json!({"schema":"rx.resident-execution-connection.v1","endpoint":endpoint,"server_name":"localhost","ca":ca_file,"certificate":cert_file,"private_key":key_file,"principal":"supervisor","installation":meta.id,"store_generation":meta.store_generation,"shared_clock_id":clock_domain,"release_digest":release})).unwrap()).unwrap();
    use rx_protocol::resident_execution as wire;
    use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity as TlsIdentity};
    let binding_hash = sha2::Sha256::digest(include_bytes!(
        "../../../spec/resident-execution/v1/binding.json"
    ))
    .to_vec();
    for who in ["observer", "host", "supervisor"] {
        let channel = Endpoint::from_shared(endpoint.clone())
            .unwrap()
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name("localhost")
                    .ca_certificate(Certificate::from_pem(ca.pem()))
                    .identity(TlsIdentity::from_pem(&certs[who].0, &certs[who].1)),
            )
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut unauthorized =
            wire::resident_execution_service_client::ResidentExecutionServiceClient::new(channel);
        let registry: Digest =
            serde_json::from_value(listing["enrollment"]["registry"].clone()).unwrap();
        let bad_registry = if who == "supervisor" {
            vec![0; 32]
        } else {
            registry.as_bytes().to_vec()
        };
        let result = unauthorized
            .open(wire::OpenSupervisor {
                peer_id: who.into(),
                peer_boot: id().to_string(),
                installation_id: meta.id.to_string(),
                store_generation: meta.store_generation.to_string(),
                shared_clock_id: clock_domain.clone(),
                release_digest: release.as_bytes().to_vec(),
                binding_hash: binding_hash.clone(),
                registry_binding: bad_registry,
            })
            .await;
        assert!(result.is_err(), "wrong role or registry accepted");
    }
    let app = rx_api::router(
        faults.clone(),
        rx_api::auth::Credentials {
            schema: "rx.local-credentials.v1".into(),
            accounts: vec![rx_api::auth::LocalAccount {
                principal: name("admin"),
                password_hash: rx_api::auth::password_hash("fixture-only").unwrap(),
            }],
        },
        rx_api::LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
    )
    .unwrap();
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
    let mut selections = BTreeMap::new();
    let mut ports = Vec::new();
    for selection in ["first", "second"] {
        let Reply::Component(component) = handle
            .call(Command::CreateComponent {
                identity: admin.clone(),
                key: id(),
                input: resident_component::Create {
                    declaration: Declaration {
                        label: name(selection),
                        catalog: catalog.clone(),
                    },
                },
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reserved.local_addr().unwrap().port();
        ports.push(reserved);
        selections.insert(
            name(selection),
            data::Selection {
                component: component.record.registration.id,
                expected_revision: component.revision,
                parameters: [
                    (name("bind"), "127.0.0.1".into()),
                    (name("port"), port.to_string()),
                ]
                .into(),
                depends_on: if selection == "second" {
                    vec![name("first")]
                } else {
                    vec![]
                },
                startup_timeout_ms: Counter(5000),
                shutdown_timeout_ms: Counter(2000),
            },
        );
    }
    let proposal = data::Propose {
        supervisor: name("supervisor"),
        environment: data::Environment::Simulation,
        profiles: vec![],
        selections,
    };
    let (status, proposed) = owner(
        &app,
        &cookie,
        "POST",
        "/api/v1/resident-executions",
        serde_json::json!({"request_key":id(),"command":proposal}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposed}");
    let initial: data::View = serde_json::from_value(proposed).unwrap();
    let assignment = initial.assignment.intent.id.clone();
    std::fs::write(&config,serde_json::to_vec(&serde_json::json!({"schema":"rx.resident-execution-runtime.v1","state_subdirectory":state_name,"connection":connection,"assignment":assignment})).unwrap()).unwrap();
    let stderr = std::fs::File::create(temp.path().join("daemon.stderr")).unwrap();
    let mut process = Process::new(&executable);
    process
        .arg("platform-run")
        .arg(&config)
        .stdout(std::process::Stdio::piped())
        .stderr(stderr)
        .kill_on_drop(true);
    let mut child = ChildOwner(process.spawn().unwrap());
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let prepared = line(&mut stdout).await;
    assert_eq!(prepared["state"], "AWAITING_OWNER_APPROVAL");
    let (status, value) = owner(
        &app,
        &cookie,
        "GET",
        &format!("/api/v1/resident-execution?id={assignment}"),
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let prepared_view: data::View = serde_json::from_value(value).unwrap();
    assert_eq!(prepared_view.assignment.phase, data::Phase::Prepared);
    assert!(!prepared_view.claims_held);
    let approval = serde_json::json!({"request_key":id(),"command":data::Approve{assignment:assignment.clone(),expected_revision:prepared_view.revision,preparation_digest:prepared_view.assignment.content.as_ref().unwrap().preparation.digest().unwrap(),start_window_ms:Counter(10_000)}});
    drop(ports);
    let (status, lost) = owner(
        &app,
        &cookie,
        "POST",
        "/api/v1/resident-executions/approve",
        approval.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{lost}");
    assert_eq!(lost["outcome_unknown"], true);
    let (status, approved) = owner(
        &app,
        &cookie,
        "POST",
        "/api/v1/resident-executions/approve",
        approval,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let mut running_report = None;
    for _ in 0..150 {
        let value = line(&mut stdout).await;
        if value["schema"] == "rx.resident-execution-runtime-status.v1"
            && value["supervisor"]["state"]["records"]
                .as_object()
                .unwrap()
                .values()
                .all(|r| r["phase"] == "PROCESS_READY")
        {
            running_report = Some(value);
            break;
        }
    }
    let running = running_report.expect("both actual processes became ready");
    for record in running["supervisor"]["state"]["records"]
        .as_object()
        .unwrap()
        .values()
    {
        assert!(record["pid"].as_u64().unwrap() > 0);
    }
    for selection in ["first", "second"] {
        let admission = &running["supervisor"]["execution_admission"][selection]["application"];
        assert_eq!(admission["state"], "REPORTED_AT_START");
        assert_eq!(admission["receipt"]["evidence"]["basis"], "LINUX_RLIMIT");
    }
    for _ in 0..100 {
        let Reply::ResidentExecution(view) = handle
            .call(Command::GetResidentExecution {
                identity: admin.clone(),
                id: assignment.clone(),
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        if view.assignment.observed.as_ref().is_some_and(|v| {
            v.observation.nodes.values().all(|n| {
                n.state == data::ObservationState::Running && n.detail.contains("ProcessReady")
            })
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Reply::ResidentExecution(current) = handle
        .call(Command::GetResidentExecution {
            identity: admin.clone(),
            id: assignment.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(current.assignment.phase, data::Phase::Running);
    assert!(current.claims_held);
    if cold {
        // Kill only the test-owned manager. Its original software children remain alive;
        // the private container is destroyed by the runner when this scene returns.
        child.0.start_kill().unwrap();
        assert!(!child.0.wait().await.unwrap().success());
        use rx_ports::Repository;
        let registry_path = std::path::PathBuf::from("/var/lib/rx-solutions")
            .join(&state_name)
            .join("registration.db");
        let mut registry = SqliteRepository::open(&registry_path).unwrap();
        let before = registry.control_events_after(Counter(0), 128).unwrap();
        registry.close().unwrap();
        let output = Process::new(&executable)
            .arg("platform-investigate")
            .arg(&config)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "cold investigation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let inspection: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            inspection["nodes"]
                .as_object()
                .unwrap()
                .values()
                .all(|node| {
                    node["finding"]["basis"] == "SCOPED_INVESTIGATION"
                        && node["finding"]["outcome"]["state"] == "MATCHING_PROCESS_PRESENT"
                        && node["recorded_outcome"] == "RUNNING"
                }),
            "{inspection}"
        );
        let mut registry = SqliteRepository::open(&registry_path).unwrap();
        assert_eq!(
            registry.control_events_after(Counter(0), 128).unwrap(),
            before
        );
        registry.close().unwrap();
        let Reply::ResidentExecution(after) = handle
            .call(Command::GetResidentExecution {
                identity: admin.clone(),
                id: assignment.clone(),
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(after.assignment, current.assignment);
        assert!(after.claims_held);
        if let Ok(path) = std::env::var("RX_RESIDENT_EXECUTION_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({
                "status":"PASS_COLD_SOURCE_INVESTIGATION", "source_inspection":inspection,
                "original_assignment_and_claims_unchanged":true, "source_history_unchanged":true,
                "process_adoption_or_termination":"NOT_PERFORMED", "physical_execution":"NOT_PERFORMED"
            })).unwrap()).unwrap();
        }
        handle.close();
        handle.closed().await;
        let _ = stop_server.send(());
        let _ = server_task.await;
        return;
    }
    let local_stop_started = std::time::Instant::now();
    if outage {
        faults.stall.store(true, Ordering::SeqCst);
        let signaled = Process::new("/bin/kill")
            .args(["-TERM", "--", &child.0.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        assert!(signaled.success());
    } else {
        let (status, stopping) = owner(&app,&cookie,"POST","/api/v1/resident-executions/stop",
            serde_json::json!({"request_key":id(),"command":data::Stop{assignment:assignment.clone(),expected_revision:current.revision}})).await;
        assert_eq!(status, StatusCode::OK, "{stopping}");
    }
    let mut local_stop_ms = None;
    let mut stopped = None;
    for _ in 0..150 {
        let value = line(&mut stdout).await;
        if value["schema"] == "rx.resident-execution-runtime-status.v1"
            && value["supervisor"]["all_exited"] == true
        {
            local_stop_ms = Some(local_stop_started.elapsed().as_millis());
        }
        if value["schema"] == "rx.resident-execution-stopped.v1" {
            stopped = Some(value);
            break;
        }
    }
    let stopped = stopped.expect("daemon stop report");
    assert_eq!(stopped["source_stopped"], true);
    assert_eq!(stopped["delivery"]["pending"], outage);
    let local_stop_ms =
        local_stop_ms.expect("local owned-child stop observed before delivery drain");
    assert!(
        local_stop_ms < 2000,
        "local stop blocked by reporting: {local_stop_ms}ms"
    );
    let exit = tokio::time::timeout(Duration::from_secs(10), child.0.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(
        exit.success(),
        "{}",
        std::fs::read_to_string(temp.path().join("daemon.stderr")).unwrap()
    );
    let Reply::ResidentExecution(final_view) = handle
        .call(Command::GetResidentExecution {
            identity: admin.clone(),
            id: assignment.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    if outage {
        assert!(final_view.claims_held);
    } else {
        assert_eq!(final_view.assignment.phase, data::Phase::Exited);
        assert!(!final_view.claims_held);
    }
    use rx_ports::Repository;
    let registry_path = std::path::PathBuf::from("/var/lib/rx-solutions")
        .join(&state_name)
        .join("registration.db");
    let mut source = SqliteRepository::open(&registry_path).unwrap();
    let before = source.control_events_after(Counter(0), 128).unwrap();
    source.close().unwrap();
    faults.stall.store(false, Ordering::SeqCst);
    let again = Process::new(&executable)
        .arg("platform-run")
        .arg(&config)
        .output()
        .await
        .unwrap();
    assert!(
        !again.status.success(),
        "original assignment replayed after daemon exit"
    );
    let mut source = SqliteRepository::open(&registry_path).unwrap();
    assert_eq!(
        source.control_events_after(Counter(0), 128).unwrap(),
        before
    );
    source.close().unwrap();
    let journal = registry_path
        .parent()
        .unwrap()
        .join("platform-runs")
        .join(assignment.as_str())
        .join("delivery.db");
    let mut store = SqliteRepository::open(&journal).unwrap();
    let (_, rows) = store.snapshot().unwrap();
    store.close().unwrap();
    let delivery_row = rows
        .iter()
        .find(|r| r.key.as_str() == "resident-execution/delivery")
        .unwrap();
    assert_eq!(delivery_row.document.value["pending"].is_object(), outage);
    assert!(
        delivery_row.document.value["latest"]["nodes"]
            .as_object()
            .unwrap()
            .values()
            .all(|n| n["state"] == "EXITED")
    );
    assert_eq!(final_view.assignment.intent, initial.assignment.intent);
    assert!(!faults.lose_observation.load(Ordering::SeqCst));
    let inspection_output = Process::new(&executable)
        .arg("platform-investigate")
        .arg(&config)
        .output()
        .await
        .unwrap();
    assert!(
        inspection_output.status.success(),
        "source investigation: {}",
        String::from_utf8_lossy(&inspection_output.stderr)
    );
    let inspection: serde_json::Value = serde_json::from_slice(&inspection_output.stdout).unwrap();
    assert_eq!(
        inspection["schema"],
        "rx.resident-execution-source-inspection.v1"
    );
    assert_eq!(inspection["assignment"], assignment.to_string());
    assert_ne!(
        inspection["original_peer"]["id"],
        inspection["current_peer"]["id"]
    );
    assert!(
        inspection["nodes"]
            .as_object()
            .unwrap()
            .values()
            .all(|node| {
                node["finding"]["basis"] == "RECORDED_DIRECT_CHILD_EXIT"
                    && node["recorded_outcome"] == "EXITED"
                    && !node["residuals"].as_array().unwrap().is_empty()
            })
    );
    assert!(
        inspection["operating_permission"]
            .as_str()
            .unwrap()
            .starts_with("NOT_GRANTED")
    );
    let mut source = SqliteRepository::open(&registry_path).unwrap();
    assert_eq!(
        source.control_events_after(Counter(0), 128).unwrap(),
        before
    );
    source.close().unwrap();
    let mut store = SqliteRepository::open(&journal).unwrap();
    assert_eq!(
        store.snapshot().unwrap().1,
        rows,
        "inspection rewrote original delivery"
    );
    store.close().unwrap();
    let Reply::ResidentExecution(after_inspection) = handle
        .call(Command::GetResidentExecution {
            identity: admin.clone(),
            id: assignment.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(after_inspection.assignment, final_view.assignment);
    assert_eq!(after_inspection.claims_held, final_view.claims_held);
    if let Ok(path) = std::env::var("RX_RESIDENT_EXECUTION_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"status":"PASS_P_ASSIGNED_ACTUAL_SOFTWARE_EXECUTION","listing":listing,"prepared":prepared,"approved":approved,"running":running,"stopped":stopped,"final":final_view,"approval_reply_loss_recovered":true,"observation_reply_loss_recovered":true,"physical_execution":"NOT_PERFORMED","functional_work_permission":"NOT_GRANTED","outage":outage,"local_stop_ms":local_stop_ms,"original_assignment_restart_refused":true,"source_history_unchanged_after_replay_attempt":true,"delivery":delivery_row.document.value,"source_inspection":inspection,"inspection_preserved_source_outbox_and_platform_outcomes":true})).unwrap()).unwrap();
    }
    handle.close();
    handle.closed().await;
    let _ = stop_server.send(());
    let _ = server_task.await;
}
