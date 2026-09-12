//! Real TLS sockets and SQLite. Read/reconnect fixtures never send device commands.
#[path = "support/checkpoint.rs"]
mod checkpoint;
#[path = "support/decision_worker.rs"]
mod decision_worker;
#[path = "support/handover.rs"]
mod handover;
#[path = "support/operator_read.rs"]
mod operator_read;
#[path = "support/production_worker.rs"]
mod production_worker;
#[path = "support/service_worker.rs"]
mod service_worker;
mod support;
use rx_api::grpc::{Configuration, PlatformIngress, TlsMaterial};
use rx_application::*;
use rx_domain::{canonical, types::*};
use rx_protocol::{base, cell};
use rx_runtime::{
    application::{Application, ApplicationPort, CallFuture, Command, Handle, Reply},
    writer::{Status, Writer, WriterError},
};
use rx_storage::SqliteRepository;
use std::{collections::BTreeMap, sync::Arc};
use tonic::transport::{
    Certificate as TlsCertificate, Channel, ClientTlsConfig, Identity as TlsIdentity,
};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
struct TestClock(Arc<std::sync::atomic::AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(self.0.load(std::sync::atomic::Ordering::SeqCst)),
        }
    }
}
struct SimulationOnly;
fn simulation_report() -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([9; 32]),
        schema_id: name("rx.validation.simulation.v1"),
        size_bytes: Counter(1),
    }
}
impl QualificationAuthority for SimulationOnly {
    fn verify(&self, c: &CellConfiguration, e: &[ArtifactRef], d: &[Digest]) -> bool {
        c.environment == Environment::Simulation
            && e == [simulation_report()]
            && d.len() == 4
            && [
                c.definition.sha256,
                c.envelope.sha256,
                c.recipe.sha256,
                c.site_config_digest,
            ]
            .iter()
            .all(|v| d.contains(v))
    }
}
fn manifest(text: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let json: serde_json::Value = serde_json::from_str(text).unwrap();
    sha2::Sha256::digest(canonical::bytes(&json).unwrap()).to_vec()
}
async fn connect(uri: &str, ca: &str, certificate: &str, key: &str) -> Channel {
    Channel::from_shared(uri.to_string())
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(TlsCertificate::from_pem(ca))
                .identity(TlsIdentity::from_pem(certificate, key)),
        )
        .unwrap()
        .connect()
        .await
        .unwrap()
}
fn call(session: &base::Session) -> base::CallContext {
    base::CallContext {
        session_id: session.session_id.clone(),
        call_id: id().to_string(),
        request_key: None,
        expected_revision: None,
    }
}
#[tokio::test]
async fn executor_run_reads_require_both_negotiations_and_the_same_registered_certificate() {
    run_fixture(false, false, None, None).await;
}
#[tokio::test]
async fn remote_executor_allocates_and_submits_after_a_simulated_operator_start() {
    run_fixture(true, false, None, None).await;
}
#[tokio::test]
#[ignore = "tools/test_executor_read_e2e.sh supplies the separate solutions reader"]
async fn solutions_reader_restores_a_new_session_without_restoring_run_authority() {
    run_fixture(true, true, None, None).await;
}
#[tokio::test]
#[ignore = "tools/test_executor_worker_e2e.sh supplies S worker and pinned C++ request fixture"]
async fn durable_s_worker_handles_real_bt_requests_and_preserves_crash_boundaries() {
    for mode in [
        "RUN",
        "CRASH_BEFORE_SUBMIT",
        "CRASH_AFTER_SUBMIT",
        "LOST_RESOLVE_REPLY",
        "LOST_SUBMIT_REPLY",
        "PAUSE",
        "LOST_PAUSE_REPLY",
        "HANDOVER",
        "LOST_RECONCILE_REPLY",
        "CHECKPOINT_BRANCH",
        "CHECKPOINT_WAIT",
        "CHECKPOINT_LOST_REPLY",
        "CHECKPOINT_CRASH_BEFORE",
        "CHECKPOINT_CRASH_AFTER",
        "CHECKPOINT_EXPIRED",
        "CHECKPOINT_WAIT_PENDING",
        "CHECKPOINT_WAIT_TIMEOUT",
        "CHECKPOINT_BRANCH_FALSE",
        "SERVICE_ACTIVE",
        "SERVICE_BEFORE_ASSIGN",
        "SERVICE_LOST_PAUSE",
        "SERVICE_CRASH_STOP",
        "SERVICE_STORE_FAULT",
        "SERVICE_PAUSE_UNAVAILABLE",
        "PRODUCTION_RUN",
        "PRODUCTION_LOST_BEGIN",
        "PRODUCTION_LOST_COMPLETE",
    ] {
        run_fixture(true, false, Some(mode), None).await;
    }
}
#[tokio::test]
async fn branch_checkpoint_preparation_and_lost_commit_reply_use_the_frozen_wire_contract() {
    run_fixture(true, false, None, Some("BRANCH")).await;
}
#[tokio::test]
async fn wait_checkpoint_start_and_resolution_use_the_frozen_wire_contract() {
    run_fixture(true, false, None, Some("WAIT")).await;
}
async fn run_fixture(
    mutate: bool,
    read_client: bool,
    worker_mode: Option<&str>,
    checkpoint_mode: Option<&str>,
) {
    use rcgen::*;
    use sha2::Digest as _;
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("p.db");
    let clock_ticks = Arc::new(std::sync::atomic::AtomicU64::new(1000));
    let engine_clock = clock_ticks.clone();
    let clock_directory = directory.path().join("clock");
    std::fs::create_dir(&clock_directory).unwrap();
    let clock_file = clock_directory.join("now.json");
    publish_clock(&clock_file, 1000);
    let decision_mode = worker_mode.is_some_and(|m| m.starts_with("CHECKPOINT_"));
    let production_mode = worker_mode.is_some_and(|m| m.starts_with("PRODUCTION_"));
    let service_mode =
        worker_mode.is_some_and(|m| m.starts_with("SERVICE_") || m.starts_with("PRODUCTION_"));
    let mut base_configuration = support::configuration("cell/a");
    if matches!(worker_mode, Some("HANDOVER" | "LOST_RECONCILE_REPLY")) || production_mode {
        base_configuration.steps[0].intent.kind = rx_domain::intent::Kind::FiniteAction;
        base_configuration.steps[0].intent.body =
            rx_domain::intent::Body::Program(rx_domain::intent::ProgramGoal {
                program: ArtifactRef {
                    sha256: Digest::from_bytes([70; 32]),
                    schema_id: name("rx.sim.program.v1"),
                    size_bytes: Counter(1),
                },
                parameter_set: ArtifactRef {
                    sha256: Digest::from_bytes([71; 32]),
                    schema_id: name("rx.sim.parameters.v1"),
                    size_bytes: Counter(1),
                },
            });
        base_configuration.steps[0].completion = CompletionRule::Native {
            schema: name("rx.sim.completed.v1"),
            success: vec![Integer(0)],
            failure: vec![Integer(1)],
            postconditions: vec![],
        };
    }
    let mut configuration = if decision_mode {
        checkpoint::configuration(
            base_configuration,
            if worker_mode.unwrap().contains("WAIT") {
                "WAIT"
            } else {
                "BRANCH"
            },
        )
    } else if let Some(kind) = checkpoint_mode {
        checkpoint::configuration(base_configuration, kind)
    } else if read_client || worker_mode.is_some() {
        graph_configuration(base_configuration)
    } else {
        support::configuration("cell/a")
    };
    if matches!(
        worker_mode,
        Some("CHECKPOINT_WAIT_PENDING" | "CHECKPOINT_WAIT_TIMEOUT" | "CHECKPOINT_BRANCH_FALSE")
    ) {
        let process = configuration.process.as_mut().unwrap();
        let rx_domain::condition::Condition::Eq { expected, .. } =
            process.conditions.get_mut(&name("ready")).unwrap()
        else {
            panic!("fixture condition")
        };
        *expected = TypedValue::Boolean(false);
        configuration.recipe.sha256 =
            rx_process_contract::frontier::resolved_digest(process).unwrap();
        configuration.recipe.size_bytes = Counter(canonical::bytes(process).unwrap().len() as u64);
    }
    let config = configuration.clone();
    let admin_session = id();
    let setup_admin_session = admin_session.clone();
    let host_session = id();
    let setup_host_session = host_session.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(db)?,
            TestClock(engine_clock),
            SimulationOnly,
            id(),
            Principal {
                id: name("admin"),
                client_namespace: name("admin"),
                roles: [
                    Role::AccountAdmin,
                    Role::Engineer,
                    Role::Operator,
                    Role::RecoveryLead,
                    Role::Verifier,
                ]
                .into_iter()
                .collect(),
                cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
                active: true,
            },
        )?;
        let session = engine.authenticated_session(
            &name("admin"),
            setup_admin_session,
            TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let admin = Identity {
            principal: name("admin"),
            session: session.id,
            terminal: None,
        };
        engine.put_principal(
            &admin,
            Principal {
                id: name("executor"),
                client_namespace: name("executor"),
                roles: [Role::Executor].into_iter().collect(),
                cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
                active: true,
            },
            None,
        )?;
        engine.put_principal(
            &admin,
            Principal {
                id: name("operator-api"),
                client_namespace: name("operator-api"),
                roles: [Role::OperatorApi, Role::Observer].into_iter().collect(),
                cells: [name("cell/a")].into_iter().collect(),
                active: true,
            },
            None,
        )?;
        engine.install_cell(&admin, config.clone())?;
        engine.create_run(
            &admin,
            id().as_str(),
            CreateRun {
                cell: config.id.clone(),
                recipe_digest: config.recipe.sha256,
                site_config_digest: config.site_config_digest,
                expected_cell: Counter(1),
            },
        )?;
        let mut other = support::configuration("cell/b");
        other.executor = name("other-executor");
        other.definition.sha256 = Digest::from_bytes([51; 32]);
        engine.install_cell(&admin, other.clone())?;
        engine.create_run(
            &admin,
            id().as_str(),
            CreateRun {
                cell: other.id,
                recipe_digest: other.recipe.sha256,
                site_config_digest: other.site_config_digest,
                expected_cell: Counter(1),
            },
        )?;
        if mutate {
            prepare_simulated_host(&mut engine, &admin, &config, setup_host_session)?;
        }
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let handle = Handle::new(writer);
    let Reply::Installation(installation) = handle.call(Command::Installation).await.unwrap()
    else {
        panic!("installation")
    };
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let leaf = |label: &str, usage: ExtendedKeyUsagePurpose| {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![label.into()]).unwrap();
        params.extended_key_usages = vec![usage];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (params.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let (executor, executor_key) = leaf("executor", ExtendedKeyUsagePurpose::ClientAuth);
    let (alternate, alternate_key) = leaf("alternate", ExtendedKeyUsagePurpose::ClientAuth);
    let (operator_api, operator_key) = leaf("operator-api", ExtendedKeyUsagePurpose::ClientAuth);
    let (unregistered, unregistered_key) =
        leaf("unregistered", ExtendedKeyUsagePurpose::ClientAuth);
    let fingerprint =
        |cert: &Certificate| Digest::from_bytes(sha2::Sha256::digest(cert.der().as_ref()).into());
    let release = Digest::from_bytes([8; 32]);
    let ingress = PlatformIngress::new(
        Arc::new(LoseWorkerReply {
            handle: handle.clone(),
            resolve: worker_mode == Some("LOST_RESOLVE_REPLY"),
            submit: worker_mode == Some("LOST_SUBMIT_REPLY"),
            pause: matches!(worker_mode, Some("LOST_PAUSE_REPLY" | "SERVICE_LOST_PAUSE")),
            pause_unavailable: worker_mode == Some("SERVICE_PAUSE_UNAVAILABLE"),
            begin_part: worker_mode == Some("PRODUCTION_LOST_BEGIN"),
            complete_part: worker_mode == Some("PRODUCTION_LOST_COMPLETE"),
            reconcile: worker_mode == Some("LOST_RECONCILE_REPLY"),
            checkpoint: checkpoint_mode.is_some() || worker_mode == Some("CHECKPOINT_LOST_REPLY"),
            expire: worker_mode == Some("CHECKPOINT_EXPIRED"),
            timeout: worker_mode == Some("CHECKPOINT_WAIT_TIMEOUT"),
            clock: clock_ticks.clone(),
            clock_file: clock_file.clone(),
            first: std::sync::atomic::AtomicBool::new(true),
        }),
        Configuration {
            installation: installation.clone(),
            release_digest: release,
            allowed_certificates: BTreeMap::from([
                (fingerprint(&executor), name("executor")),
                (fingerprint(&alternate), name("executor")),
                (fingerprint(&operator_api), name("operator-api")),
            ]),
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("https://{}", listener.local_addr().unwrap());
    let tls = TlsMaterial {
        server_certificate_pem: server.pem().into_bytes(),
        server_key_pem: server_key.serialize_pem().into_bytes(),
        client_ca_pem: ca.pem().into_bytes(),
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        ingress
            .serve(listener, tls, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let channel = connect(
        &uri,
        &ca.pem(),
        &executor.pem(),
        &executor_key.serialize_pem(),
    )
    .await;
    let alternate_channel = connect(
        &uri,
        &ca.pem(),
        &alternate.pem(),
        &alternate_key.serialize_pem(),
    )
    .await;
    let unregistered_channel = connect(
        &uri,
        &ca.pem(),
        &unregistered.pem(),
        &unregistered_key.serialize_pem(),
    )
    .await;
    let base_hash = manifest(include_str!(
        "../../../spec/contracts/v1.0/protocol_manifest.json"
    ));
    let cell_hash = manifest(include_str!(
        "../../../spec/cell_operations/v1.0/protocol_manifest.json"
    ));
    let hello = base::PeerHello {
        installation_id: installation.id.to_string(),
        store_generation: installation.store_generation.to_string(),
        peer_id: "executor".into(),
        role: base::Role::Executor as i32,
        boot_id: id().to_string(),
        release_digest: release.as_bytes().to_vec(),
        supported_versions: vec![base::Version {
            major: 1,
            minor: 0,
            schema_hash: base_hash.clone(),
        }],
        shared_clock_id: installation.clock_id.clone(),
        journal_id: None,
        last_seq: None,
    };
    if !mutate && worker_mode.is_none() {
        let operator_channel = connect(
            &uri,
            &ca.pem(),
            &operator_api.pem(),
            &operator_key.serialize_pem(),
        )
        .await;
        operator_read::probe(
            operator_channel,
            hello.clone(),
            configuration.definition.sha256.as_bytes().to_vec(),
            cell_hash.clone(),
        )
        .await;
    }
    let mut worker_child = None;
    let mut worker_config = None;
    let worker_output = directory.path().join("worker-output");
    let worker_trigger = directory.path().join("worker-trigger");
    if let Some(mode) = worker_mode {
        let admin = Identity {
            principal: name("admin"),
            session: admin_session.clone(),
            terminal: None,
        };
        let Reply::Overview(overview) = handle.call(Command::Overview(admin)).await.unwrap() else {
            panic!("operator overview")
        };
        let assigned_run = overview
            .cells
            .iter()
            .find(|c| c.cell.value.id == configuration.id)
            .unwrap()
            .runs[0]
            .value
            .id
            .clone();
        for (file, value) in [
            ("worker-ca.pem", ca.pem()),
            ("worker.pem", executor.pem()),
            ("worker-key.pem", executor_key.serialize_pem()),
        ] {
            std::fs::write(directory.path().join(file), value).unwrap();
        }
        let mut config = serde_json::json!({"uri":uri,"server_name":"localhost","ca":directory.path().join("worker-ca.pem"),"certificate":directory.path().join("worker.pem"),"key":directory.path().join("worker-key.pem"),
            "pin":{"principal":"executor","peer_boot":hello.boot_id,"installation":installation.id,"store_generation":installation.store_generation,"release":release,"clock_id":"test-clock","cell":"cell/a","definition":configuration.definition.sha256},
            "run":assigned_run,"visit":"1","ticks":"1000","output":worker_output,"journal":directory.path().join("executor-journal.db"),"ready":directory.path().join("worker-ready.json"),"trigger":worker_trigger,
            "cpp_image":std::env::var("RX_BT_REQUEST_IMAGE").expect("pinned C++ image"),"mode":if mode=="LOST_PAUSE_REPLY" {"PAUSE"} else if mode=="LOST_RECONCILE_REPLY" {"HANDOVER"} else if mode.starts_with("LOST_") {"RUN"} else {mode}});
        if decision_mode || service_mode {
            config["scenario"] = serde_json::json!(mode);
            config["clock_file"] = serde_json::json!(clock_file);
        }
        if service_mode {
            config["stop_file"] = serde_json::json!(directory.path().join("service-stop"));
            config["status_file"] = serde_json::json!(directory.path().join("service-status.json"));
        }
        let path = directory.path().join("worker-config.json");
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let child = std::process::Command::new(
            std::env::var(if service_mode {
                "RX_EXECUTOR_SERVICE_FIXTURE"
            } else if decision_mode {
                "RX_EXECUTOR_DECISION_FIXTURE"
            } else {
                "RX_EXECUTOR_WORKER_FIXTURE"
            })
            .expect("worker fixture executable"),
        )
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
        worker_child = Some(child);
        worker_config = Some(config);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while !directory.path().join("worker-ready.json").exists() {
            assert!(
                worker_child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "worker exited before ready"
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "worker ready timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    let mut sessions = base::session_service_client::SessionServiceClient::new(channel.clone());
    let session = sessions.open(hello.clone()).await.unwrap().into_inner();
    let mut wrong_role = hello.clone();
    wrong_role.role = base::Role::Host as i32;
    wrong_role.journal_id = Some(id().to_string());
    wrong_role.last_seq = Some(0);
    assert_eq!(
        sessions.open(wrong_role).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );

    assert_eq!(
        sessions
            .open(hello.clone())
            .await
            .unwrap()
            .into_inner()
            .session_id,
        session.session_id
    );
    assert_eq!(
        base::session_service_client::SessionServiceClient::new(unregistered_channel)
            .open(hello.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let identity = Identity {
        principal: name("executor"),
        session: Id::new(&session.session_id).unwrap(),
        terminal: None,
    };
    let Reply::Overview(overview) = handle
        .call(Command::Overview(identity.clone()))
        .await
        .unwrap()
    else {
        panic!("overview")
    };
    let run = overview
        .cells
        .iter()
        .find(|c| c.cell.value.id == configuration.id)
        .unwrap()
        .runs[0]
        .value
        .id
        .clone();
    let other_run = overview
        .cells
        .iter()
        .find(|c| c.cell.value.id == name("cell/b"))
        .unwrap()
        .runs[0]
        .value
        .id
        .clone();
    let request = base::RunRequest {
        context: Some(call(&session)),
        run_id: Some(run.to_string()),
        recipe_digest: vec![],
        site_config_digest: vec![],
    };
    let mut workflow = base::workflow_service_client::WorkflowServiceClient::new(channel.clone());
    if worker_mode.is_none() {
        assert_eq!(
            workflow.get_run(request.clone()).await.unwrap_err().code(),
            tonic::Code::FailedPrecondition
        );
    }

    let cell_hello = cell::CellHello {
        base_session_id: session.session_id.clone(),
        peer_id: "executor".into(),
        base_manifest_hash: base_hash,
        cell_manifest_hash: cell_hash,
        cell_definition_digest: configuration.definition.sha256.as_bytes().to_vec(),
        shared_clock_id: installation.clock_id,
    };
    let mut cells = cell::cell_service_client::CellServiceClient::new(channel.clone());
    let mut mismatch = cell_hello.clone();
    mismatch.cell_manifest_hash[0] ^= 1;
    assert_eq!(
        cells.open(mismatch).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let assignment_request = rx_protocol::assignment::InspectCell {
        context: Some(call(&session)),
        cell_id: "cell/a".into(),
        binding_hash: manifest(include_str!("../../../spec/assignment/v1/binding.json")),
    };
    let mut assignments = rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(channel.clone());
    if worker_mode.is_none() {
        assert_eq!(
            assignments
                .inspect(assignment_request.clone())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    cells.open(cell_hello.clone()).await.unwrap();
    let raw_assignment = assignments
        .inspect(assignment_request.clone())
        .await
        .unwrap()
        .into_inner();
    let discovery: rx_process_contract::assignment::View =
        serde_json::from_slice(&raw_assignment.payload).unwrap();
    rx_process_contract::assignment::validate(&discovery).unwrap();
    assert_eq!(discovery.cell.as_str(), "cell/a");
    assert_eq!(
        discovery.cardinality,
        rx_process_contract::assignment::Cardinality::None
    );
    {
        use sha2::Digest as _;
        let reference = raw_assignment.reference.unwrap();
        assert_eq!(
            reference.sha256,
            sha2::Sha256::digest(&raw_assignment.payload).to_vec()
        );
        assert_eq!(reference.size_bytes, raw_assignment.payload.len() as u64);
        assert_eq!(reference.schema_id, rx_process_contract::assignment::SCHEMA);
    }
    let mut wrong_assignment = assignment_request.clone();
    wrong_assignment.binding_hash[0] ^= 1;
    assert_eq!(
        assignments
            .inspect(wrong_assignment)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    let mut wrong_assignment = assignment_request.clone();
    wrong_assignment.context.as_mut().unwrap().request_key = Some(id().to_string());
    assert_eq!(
        assignments
            .inspect(wrong_assignment)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    let mut wrong_assignment = assignment_request.clone();
    wrong_assignment.cell_id = "cell/b".into();
    assert_eq!(
        assignments
            .inspect(wrong_assignment)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    let mut foreign_assignments = rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(alternate_channel.clone());
    assert_eq!(
        foreign_assignments
            .inspect(assignment_request.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    let context = cells
        .inspect(cell::CellCall {
            context: Some(call(&session)),
            cell_id: "cell/a".into(),
            expected_cell_revision: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(context.cell_id, "cell/a");
    assert_eq!(context.mode, cell::OperatingMode::Setup as i32);
    let view = workflow
        .get_run(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(view.run_id, run.to_string());
    assert_eq!(view.state, base::RunState::Prepared as i32);
    assert_eq!(view.checkpoint.as_ref().unwrap().revision, view.revision);
    assert!(view.executor_session_id.is_none());
    let mut bad_revision = request.clone();
    bad_revision.context.as_mut().unwrap().expected_revision = Some(1);
    assert_eq!(
        workflow.get_run(bad_revision).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut bad_digest = request.clone();
    bad_digest.recipe_digest = vec![1; 32];
    assert_eq!(
        workflow.get_run(bad_digest).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut other = request.clone();
    other.run_id = Some(other_run.to_string());
    assert_eq!(
        workflow.get_run(other).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let mut alternate_workflow =
        base::workflow_service_client::WorkflowServiceClient::new(alternate_channel);
    assert_eq!(
        alternate_workflow
            .get_run(request.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        workflow
            .start_run(request.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    if mutate && worker_mode != Some("SERVICE_BEFORE_ASSIGN") {
        let Reply::Session(bound) = handle
            .call(Command::OpenTerminalUserSession {
                principal: name("admin"),
                id: id(),
                ttl_ns: Counter(3_600_000_000_000),
                certificate: Digest::from_bytes([77; 32]),
            })
            .await
            .unwrap()
        else {
            panic!("terminal session")
        };
        let admin = Identity {
            principal: name("admin"),
            session: bound.id,
            terminal: Some((name("test/panel"), Digest::from_bytes([77; 32]))),
        };
        let host = Identity {
            principal: name("host/sim"),
            session: host_session.clone(),
            terminal: None,
        };
        run_mutations(
            &handle,
            channel.clone(),
            &mut cells,
            &mut workflow,
            &session,
            &configuration,
            &run,
            &admin,
            &host,
            (worker_mode.is_some() || checkpoint_mode.is_some())
                .then_some(worker_trigger.as_path()),
            !read_client,
            production_mode,
        )
        .await;
    }
    if mutate && !read_client && worker_mode.is_none() && checkpoint_mode.is_none() {
        let open = cell::OpenCaseRequest {
            call: Some(cell::CellCall {
                context: Some(base::CallContext {
                    request_key: Some(id().to_string()),
                    ..call(&session)
                }),
                cell_id: configuration.id.to_string(),
                expected_cell_revision: Some(1),
            }),
            r#type: cell::CaseType::FaultRecovery as i32,
            scopes: vec![],
            procedure: Some(base::ArtifactRef {
                sha256: vec![95; 32],
                schema_id: "rx.test.procedure.v1".into(),
                size_bytes: 1,
            }),
            lead: "admin".into(),
            operation_ids: vec![],
            material_ids: vec![],
        };
        let case = cells.open_case(open.clone()).await.unwrap().into_inner();
        assert_eq!(case.state, cell::CaseState::ContainmentPending as i32);
        assert!(!case.block_ids.is_empty());
        assert!(case.participants.is_empty());
        assert_eq!(cells.open_case(open).await.unwrap().into_inner(), case);
        let read = cells
            .get_case(cell::GetCaseRequest {
                call: Some(cell::CellCall {
                    context: Some(call(&session)),
                    cell_id: configuration.id.to_string(),
                    expected_cell_revision: None,
                }),
                case_id: case.case_id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(read.case_id, case.case_id);
        assert_eq!(read.state, case.state);
        assert_eq!(
            cells
                .record_procedure(cell::RecordProcedureRequest::default())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    if let Some(kind) = checkpoint_mode {
        checkpoint::exercise(channel.clone(), &session, &run, &configuration, kind).await;
    }
    if production_mode {
        production_worker::finish(
            &handle,
            &directory,
            worker_child.take().unwrap(),
            worker_config.take().unwrap(),
            &worker_output,
            worker_mode.unwrap(),
            Identity {
                principal: name("admin"),
                session: admin_session.clone(),
                terminal: None,
            },
            Identity {
                principal: name("host/sim"),
                session: host_session.clone(),
                terminal: None,
            },
            &run,
        )
        .await;
        stop.send(()).unwrap();
        task.await.unwrap();
        handle.close();
        handle.closed().await;
        return;
    }
    if service_mode {
        service_worker::finish(
            &handle,
            &directory,
            worker_child.take().unwrap(),
            worker_config.take().unwrap(),
            &worker_output,
            worker_mode.unwrap(),
            Identity {
                principal: name("admin"),
                session: admin_session.clone(),
                terminal: None,
            },
        )
        .await;
        stop.send(()).unwrap();
        task.await.unwrap();
        handle.close();
        handle.closed().await;
        return;
    }
    if decision_mode {
        decision_worker::finish(
            &handle,
            &directory,
            worker_child.take().unwrap(),
            worker_config.take().unwrap(),
            &worker_output,
            worker_mode.unwrap(),
        )
        .await;
        stop.send(()).unwrap();
        task.await.unwrap();
        handle.close();
        handle.closed().await;
        return;
    }
    if let Some(mode) = worker_mode {
        let mut child = worker_child.take().unwrap();
        if matches!(mode, "HANDOVER" | "LOST_RECONCILE_REPLY") {
            handover::complete_and_release(
                &handle,
                Identity {
                    principal: name("admin"),
                    session: admin_session.clone(),
                    terminal: None,
                },
                Identity {
                    principal: name("host/sim"),
                    session: host_session.clone(),
                    terminal: None,
                },
                &worker_output,
                &mut child,
            )
            .await;
        }
        wait_child(&mut child).await;
        let result = child.wait_with_output().unwrap();
        let expected = match mode {
            "CRASH_BEFORE_SUBMIT" => 73,
            "CRASH_AFTER_SUBMIT" => 74,
            _ => 0,
        };
        assert_eq!(
            result.status.code(),
            Some(expected),
            "worker failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let expected_work = usize::from(mode != "CRASH_BEFORE_SUBMIT");
        let Reply::Overview(overview) = handle
            .call(Command::Overview(Identity {
                principal: name("admin"),
                session: admin_session.clone(),
                terminal: None,
            }))
            .await
            .unwrap()
        else {
            panic!("overview")
        };
        assert_eq!(
            overview.cells.iter().map(|c| c.work.len()).sum::<usize>(),
            expected_work
        );
        let source_file = if !mode.starts_with("CRASH_") {
            "report.json"
        } else {
            "boundary.json"
        };
        let before: serde_json::Value =
            serde_json::from_slice(&std::fs::read(worker_output.join(source_file)).unwrap())
                .unwrap();
        let original_key = if !mode.starts_with("CRASH_") {
            before["request"]["key"].clone()
        } else {
            before["key"].clone()
        };
        let mut config = worker_config.take().unwrap();
        if matches!(mode, "PAUSE" | "LOST_PAUSE_REPLY") {
            let reply = &before["pause_request"];
            assert_eq!(reply["send"], "EMIT_ENTERED");
            assert_eq!(
                reply["resolution"]["state"],
                if mode == "PAUSE" { "REPLY" } else { "PENDING" }
            );
            config["pause_key"] = reply["key"].clone();
            let Reply::VersionedRun(_, paused) = handle
                .call(Command::InspectRun {
                    identity: Identity {
                        principal: name("admin"),
                        session: admin_session.clone(),
                        terminal: None,
                    },
                    run: run.clone(),
                })
                .await
                .unwrap()
            else {
                panic!("paused run")
            };
            assert_eq!(paused.state, RunState::Paused);
        }
        config["mode"] = serde_json::json!("RECOVER");
        config["pin"]["peer_boot"] = serde_json::json!(id());
        let recovered = directory.path().join("worker-recovered");
        config["output"] = serde_json::json!(recovered);
        config["ready"] = serde_json::json!(directory.path().join("worker-recovered-ready.json"));
        let path = directory.path().join("worker-recover-config.json");
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let mut child =
            std::process::Command::new(std::env::var("RX_EXECUTOR_WORKER_FIXTURE").unwrap())
                .arg(path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
        wait_child(&mut child).await;
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "recovery: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(recovered.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["work_count"], expected_work);
        assert_eq!(report["request"]["key"], original_key);
        assert_eq!(report["request"]["send"], "EMIT_ENTERED");
        assert_eq!(
            report["request"]["resolution"]["state"],
            if matches!(
                mode,
                "RUN"
                    | "LOST_RESOLVE_REPLY"
                    | "PAUSE"
                    | "LOST_PAUSE_REPLY"
                    | "HANDOVER"
                    | "LOST_RECONCILE_REPLY"
            ) {
                "REPLY"
            } else {
                "PENDING"
            }
        );
        if mode == "LOST_RESOLVE_REPLY" {
            assert_eq!(
                report["activation_request"]["resolution"]["state"],
                "PENDING"
            );
        }
        if matches!(mode, "PAUSE" | "LOST_PAUSE_REPLY") {
            assert_eq!(
                report["pause_request"]["key"],
                before["pause_request"]["key"]
            );
            assert_eq!(
                report["pause_request"]["resolution"]["state"],
                before["pause_request"]["resolution"]["state"]
            );
        }
        if matches!(mode, "HANDOVER" | "LOST_RECONCILE_REPLY") {
            assert_eq!(
                report["handover_request"]["key"],
                before["handover_request"]["key"]
            );
            assert_eq!(report["handover_request"]["send"], "EMIT_ENTERED");
            assert_eq!(
                report["handover_request"]["resolution"]["state"],
                if mode == "HANDOVER" {
                    "REPLY"
                } else {
                    "PENDING"
                }
            );
            assert_eq!(report["release_observed"], true);
        }
        let Reply::Overview(after) = handle
            .call(Command::Overview(Identity {
                principal: name("admin"),
                session: admin_session.clone(),
                terminal: None,
            }))
            .await
            .unwrap()
        else {
            panic!("overview")
        };
        assert_eq!(
            after.cells.iter().map(|c| c.work.len()).sum::<usize>(),
            expected_work
        );
        if let Ok(root) = std::env::var("RX_EXECUTOR_WORKER_OUTPUT") {
            let root = std::path::PathBuf::from(root).join(mode.to_ascii_lowercase());
            std::fs::create_dir(&root).unwrap();
            if matches!(mode, "PAUSE" | "LOST_PAUSE_REPLY") {
                for file in ["pause-frame.json", "pause-requests.json"] {
                    std::fs::copy(worker_output.join(file), root.join(file)).unwrap();
                }
            }
            if matches!(mode, "HANDOVER" | "LOST_RECONCILE_REPLY") {
                for file in ["handover-frame.json", "handover-requests.json"] {
                    std::fs::copy(worker_output.join(file), root.join(file)).unwrap();
                }
            }
            for file in [
                "requests.json",
                "frame.json",
                "resolved.json",
                "process.bt.xml",
                source_file,
            ] {
                std::fs::copy(worker_output.join(file), root.join(file)).unwrap();
            }
            std::fs::copy(
                recovered.join("report.json"),
                root.join("recovered-report.json"),
            )
            .unwrap();
        }
        stop.send(()).unwrap();
        task.await.unwrap();
        handle.close();
        handle.closed().await;
        return;
    }
    if read_client {
        let binding = manifest(include_str!("../../../spec/executor/v1/binding.json"));
        let mut reads =
            rx_protocol::executor::executor_read_service_client::ExecutorReadServiceClient::new(
                channel.clone(),
            );
        let query = rx_protocol::executor::SnapshotRequest {
            context: Some(call(&session)),
            run_id: run.to_string(),
            visit: 1,
            binding_hash: binding.clone(),
        };
        let reply = reads
            .get_snapshot(query.clone())
            .await
            .unwrap()
            .into_inner();
        let snapshot: ExecutionSnapshot = canonical::decode_json(&reply.payload).unwrap();
        assert!(snapshot.request_admission_allowed);
        assert_eq!(snapshot.progress.operations.len(), 1);
        let operation = snapshot
            .progress
            .operations
            .values()
            .next()
            .unwrap()
            .operation
            .id()
            .clone();
        let mut wrong = query;
        wrong.binding_hash[0] ^= 1;
        assert_eq!(
            reads.get_snapshot(wrong).await.unwrap_err().code(),
            tonic::Code::FailedPrecondition
        );
        let mut reference = reply.reference.unwrap();
        reference.size_bytes += 1;
        // A live-view payload is not a run checkpoint artifact and cannot be fetched as one.
        assert!(
            reads
                .get_artifact(rx_protocol::executor::ArtifactRequest {
                    context: Some(call(&session)),
                    run_id: run.to_string(),
                    reference: Some(reference),
                    binding_hash: binding
                })
                .await
                .is_err()
        );
        for (file, value) in [
            ("reader-ca.pem", ca.pem()),
            ("reader.pem", executor.pem()),
            ("reader-key.pem", executor_key.serialize_pem()),
        ] {
            std::fs::write(directory.path().join(file), value).unwrap();
        }
        let output = directory.path().join("reader-output");
        let config = serde_json::json!({"uri":uri,"server_name":"localhost","ca":directory.path().join("reader-ca.pem"),"certificate":directory.path().join("reader.pem"),"key":directory.path().join("reader-key.pem"),
            "pin":{"principal":"executor","peer_boot":id(),"installation":installation.id,"store_generation":installation.store_generation,"release":release,"clock_id":"test-clock","cell":"cell/a","definition":configuration.definition.sha256},
            "run":run,"visit":"1","ticks":"1000","output":output});
        let config_path = directory.path().join("reader-config.json");
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let executable = std::env::var("RX_EXECUTOR_READ_FIXTURE").expect("reader fixture path");
        let result = tokio::task::spawn_blocking(move || {
            std::process::Command::new(executable)
                .arg(config_path)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            result.status.success(),
            "reader failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let restored: rx_process_contract::execution::ExecutorState =
            canonical::decode_json(&std::fs::read(output.join("checkpoint.json")).unwrap())
                .unwrap();
        let read: ExecutionSnapshot =
            canonical::decode_json(&std::fs::read(output.join("snapshot.json")).unwrap()).unwrap();
        assert_eq!(restored.run.state, RunState::RecoveryRequired);
        assert!(!read.request_admission_allowed);
        assert_eq!(
            read.progress
                .operations
                .values()
                .next()
                .unwrap()
                .operation
                .id(),
            &operation
        );
        assert_eq!(restored.activations[0].slots[0].operation, operation);
        let frame: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("frame.json")).unwrap()).unwrap();
        assert_eq!(frame["submission_authorized"], false);
        if let Ok(target) = std::env::var("RX_EXECUTOR_FRAME_OUTPUT") {
            let target = std::path::PathBuf::from(target);
            std::fs::create_dir(&target).unwrap();
            for file in [
                "checkpoint.json",
                "snapshot.json",
                "frame.json",
                "resolved.json",
                "process.bt.xml",
            ] {
                std::fs::copy(output.join(file), target.join(file)).unwrap();
            }
        }
    }
    let mut newer = hello.clone();
    newer.boot_id = id().to_string();
    let next = sessions.open(newer).await.unwrap().into_inner();
    assert_ne!(next.session_id, session.session_id);
    assert_eq!(
        workflow.get_run(request.clone()).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        sessions.open(hello).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut next_request = request;
    next_request.context = Some(call(&next));
    assert_eq!(
        workflow
            .get_run(next_request.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    let mut next_cell = cell_hello;
    next_cell.base_session_id = next.session_id;
    cells.open(next_cell).await.unwrap();
    let after = workflow.get_run(next_request).await.unwrap().into_inner();
    assert_ne!(after.state, base::RunState::Executing as i32);
    assert!(after.executor_session_id.is_none());
    stop.send(()).unwrap();
    task.await.unwrap();
    handle.close();
    handle.closed().await;
}

fn prepare_simulated_host(
    engine: &mut Engine<SqliteRepository, TestClock, SimulationOnly>,
    admin: &Identity,
    configuration: &CellConfiguration,
    host_session: Id,
) -> rx_ports::Result<()> {
    engine.put_principal(
        admin,
        Principal {
            id: name("host/sim"),
            client_namespace: name("host/sim"),
            roles: [Role::Host].into_iter().collect(),
            cells: [configuration.id.clone()].into_iter().collect(),
            active: true,
        },
        None,
    )?;
    engine.authenticated_session(
        &name("host/sim"),
        host_session.clone(),
        TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(u64::MAX),
        },
    )?;
    let host = Identity {
        principal: name("host/sim"),
        session: host_session.clone(),
        terminal: None,
    };
    let (revision, _) = engine.inspect_cell(admin, &configuration.id)?;
    engine.qualify(
        admin,
        &configuration.id,
        revision,
        vec![simulation_report()],
        vec![
            configuration.definition.sha256,
            configuration.envelope.sha256,
            configuration.recipe.sha256,
            configuration.site_config_digest,
        ],
    )?;
    engine.put_terminal(
        admin,
        Terminal {
            id: name("test/panel"),
            certificate_digest: Digest::from_bytes([77; 32]),
            cells: [configuration.id.clone()].into_iter().collect(),
            active: true,
        },
        None,
    )?;
    let (_, cell) = engine.inspect_cell(admin, &configuration.id)?;
    let source = id();
    let registration = HostRegistration {
        id: host.principal.clone(),
        session: host_session,
        boot_id: id(),
        delivery_journal: id(),
        cell: configuration.id.clone(),
        epoch: cell.epoch,
        scopes: cell.scope_epochs,
        source_sessions: BTreeMap::from([(name("ready"), source.clone())]),
        grant: Grant {
            id: id(),
            fence: Counter(1),
            resources: configuration.steps[0].intent.resource_set.clone(),
            owner: name(engine.installation.id.as_str()),
            valid_until: TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(1_000_000_000),
            },
            ttl_ms: Counter(1000),
        },
    };
    engine.register_host(&host, registration)?;
    engine.report_fact(
        &host,
        FactRecord {
            cell: configuration.id.clone(),
            id: name("ready"),
            source_host: host.principal.clone(),
            source_generation: source,
            schema: name("boolean/v1"),
            unit: name("unitless"),
            acquired_at: TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(1000),
            },
            maximum_age_ns: Counter(1_000_000_000),
            acquisition_uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
            disputed: false,
            value: TypedValue::Boolean(true),
            evidence_id: id(),
        },
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_mutations(
    handle: &Handle<Application<SqliteRepository, TestClock, SimulationOnly>>,
    channel: Channel,
    cells: &mut cell::cell_service_client::CellServiceClient<Channel>,
    workflow: &mut base::workflow_service_client::WorkflowServiceClient<Channel>,
    session: &base::Session,
    configuration: &CellConfiguration,
    run: &Id,
    admin: &Identity,
    host: &Identity,
    worker_trigger: Option<&std::path::Path>,
    pause_at_end: bool,
    serial_production: bool,
) {
    let Reply::Cell(cell_revision, _) = handle
        .call(Command::InspectCell {
            identity: admin.clone(),
            cell: configuration.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let Reply::Attempt(attempt) = handle
        .call(Command::StartRun {
            identity: admin.clone(),
            key: id(),
            command: StartRun {
                run: run.clone(),
                envelope_digest: configuration.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
                budget_limit: Counter(if serial_production { 2 } else { 1 }),
                expected_cell: cell_revision,
                expected_run: Counter(1),
            },
        })
        .await
        .unwrap()
    else {
        panic!("attempt")
    };
    let Reply::PendingDeliveries(pending) = handle
        .call(Command::PendingDeliveries {
            after: None,
            limit: 8,
        })
        .await
        .unwrap()
    else {
        panic!("outbox")
    };
    let message = pending
        .iter()
        .find(|p| matches!(&p.payload,Delivery::Arm {attempt:target,..} if target==&attempt.id))
        .unwrap()
        .id
        .clone();
    let Reply::DeliveryPlan(plan) = handle
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: message.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("plan")
    };
    let registration = plan.registration;
    handle
        .call(Command::FinishArm {
            identity: host.clone(),
            message,
            ack: ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: registration.boot_id,
                delivery_journal: registration.delivery_journal,
                sequence: Counter(1),
                epoch: registration.epoch,
                scopes: registration.scopes,
            },
        })
        .await
        .unwrap();
    let Reply::VersionedRun(_, running) = handle
        .call(Command::InspectRun {
            identity: admin.clone(),
            run: run.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    assert_eq!(running.state, RunState::Executing);
    let current = cells
        .inspect(cell::CellCall {
            context: Some(call(session)),
            cell_id: configuration.id.to_string(),
            expected_cell_revision: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.mode, cell::OperatingMode::Automatic as i32);
    let cell_revision = Counter(current.revision);
    if serial_production {
        return;
    }
    let begin = cell::BeginPartAttemptRequest {
        call: Some(cell::CellCall {
            context: Some(base::CallContext {
                request_key: Some(id().to_string()),
                ..call(session)
            }),
            cell_id: configuration.id.to_string(),
            expected_cell_revision: Some(cell_revision.0),
        }),
        run_id: run.to_string(),
        mandate_id: running.mandate.unwrap().to_string(),
        expected_budget_revision: running.budget.unwrap().revision().0,
    };
    let part = cells
        .begin_part_attempt(begin.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(part.revision, 1);
    assert_eq!(part.ordinal, 1);
    if let Some(trigger) = worker_trigger {
        std::fs::write(trigger, b"assigned").unwrap();
        return;
    }
    assert_eq!(part.disposition, cell::PartDisposition::InProgress as i32);
    assert_eq!(
        cells
            .begin_part_attempt(begin.clone())
            .await
            .unwrap()
            .into_inner(),
        part
    );
    let mandate = begin.mandate_id.clone();
    let mut changed = begin;
    changed.mandate_id = id().to_string();
    assert_eq!(
        cells.begin_part_attempt(changed).await.unwrap_err().code(),
        tonic::Code::AlreadyExists
    );
    let request = base::RunRequest {
        context: Some(call(session)),
        run_id: Some(run.to_string()),
        recipe_digest: vec![],
        site_config_digest: vec![],
    };
    let before = workflow
        .get_run(request.clone())
        .await
        .unwrap()
        .into_inner();
    let resolve = base::ResolveActivation {
        context: Some(base::CallContext {
            request_key: Some(id().to_string()),
            expected_revision: Some(before.revision),
            ..call(session)
        }),
        run_id: run.to_string(),
        node_id: configuration.steps[0].id.to_string(),
        visit: part.ordinal,
    };
    let activation = workflow
        .resolve_activation(resolve.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        workflow
            .resolve_activation(resolve.clone())
            .await
            .unwrap()
            .into_inner(),
        activation
    );
    let mut changed = resolve.clone();
    changed.visit = 2;
    assert_eq!(
        workflow
            .resolve_activation(changed)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::AlreadyExists
    );
    let mut another_key = resolve;
    another_key.context.as_mut().unwrap().request_key = Some(id().to_string());
    assert_eq!(
        workflow
            .resolve_activation(another_key)
            .await
            .unwrap()
            .into_inner(),
        activation
    );
    let after = workflow.get_run(request).await.unwrap().into_inner();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(
        after.checkpoint.unwrap().activations,
        vec![activation.clone()]
    );
    let context = base::CallContext {
        request_key: Some(id().to_string()),
        ..call(session)
    };
    let intent = rx_protocol::json::from_slice::<base::Intent>(
        &canonical::bytes(&configuration.steps[0].intent).unwrap(),
    )
    .unwrap();
    let submission = cell::SubmitOperationRequest {
        call: Some(cell::CellCall {
            context: Some(context.clone()),
            cell_id: configuration.id.to_string(),
            expected_cell_revision: Some(cell_revision.0),
        }),
        request: Some(base::SubmitOperation {
            context: Some(context),
            intent: Some(intent),
            run_id: Some(run.to_string()),
            activation_id: Some(activation.activation_id),
            slot: Some("main".into()),
        }),
        parent: Some(cell::PermitParent {
            value: Some(cell::permit_parent::Value::MandateId(mandate)),
        }),
        part_attempt_id: Some(part.part_attempt_id),
        expected_run_revision: Some(after.revision),
        expected_case_revision: None,
    };
    let mut mismatch = submission.clone();
    mismatch
        .request
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .call_id = id().to_string();
    assert_eq!(
        cells.submit_operation(mismatch).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut mismatch = submission.clone();
    mismatch.part_attempt_id = Some(id().to_string());
    assert_eq!(
        cells.submit_operation(mismatch).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut mismatch = submission.clone();
    mismatch.request.as_mut().unwrap().run_id = Some(id().to_string());
    assert_eq!(
        cells.submit_operation(mismatch).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut mismatch = submission.clone();
    mismatch.parent = Some(cell::PermitParent {
        value: Some(cell::permit_parent::Value::MandateId(id().to_string())),
    });
    assert_eq!(
        cells.submit_operation(mismatch).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let mut stale = submission.clone();
    stale.expected_run_revision = Some(after.revision - 1);
    assert_eq!(
        cells.submit_operation(stale).await.unwrap_err().code(),
        tonic::Code::Aborted
    );
    let receipt = cells
        .submit_operation(submission.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(receipt.stage, base::ReceiptStage::Admitted as i32);
    assert_eq!(receipt.operation_revision, Some(1));
    assert!(
        receipt.journal_seq > 0 && receipt.invocation_id.is_none() && receipt.host_state.is_none()
    );
    let mut retry = submission.clone();
    let trace = id().to_string();
    retry
        .call
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .call_id = trace.clone();
    retry
        .request
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .call_id = trace;
    assert_eq!(
        cells.submit_operation(retry).await.unwrap().into_inner(),
        receipt
    );
    let mut changed = submission.clone();
    changed.expected_run_revision = Some(after.revision + 1);
    assert_eq!(
        cells.submit_operation(changed).await.unwrap_err().code(),
        tonic::Code::AlreadyExists
    );
    let mut second_key = submission.clone();
    let key = id().to_string();
    second_key
        .call
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .request_key = Some(key.clone());
    second_key
        .request
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .request_key = Some(key);
    assert_eq!(
        cells
            .submit_operation(second_key)
            .await
            .unwrap()
            .into_inner(),
        receipt
    );
    let mut operations = base::operation_service_client::OperationServiceClient::new(channel);
    assert_eq!(
        operations
            .submit(submission.request.unwrap())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    let view = operations
        .get(base::OperationRef {
            context: Some(call(session)),
            operation_id: receipt.operation_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(view.operation_id, receipt.operation_id);
    assert_eq!(view.intent_digest, receipt.intent_digest);
    assert_eq!(view.phase, base::Phase::Admitted as i32);
    assert_eq!(view.execution_knowledge, base::Knowledge::NotSent as i32);
    assert_eq!(view.outcome, base::Outcome::None as i32);
    assert_eq!(view.disposition, base::Disposition::Held as i32);
    assert!(view.evidence_ids.is_empty());
    let Reply::VersionedRun(_, run_state) = handle
        .call(Command::InspectRun {
            identity: admin.clone(),
            run: run.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    assert_eq!(run_state.part_ids.len(), 1);
    let Reply::Overview(overview) = handle.call(Command::Overview(admin.clone())).await.unwrap()
    else {
        panic!("overview")
    };
    assert_eq!(
        overview
            .cells
            .iter()
            .map(|cell| cell.work.len())
            .sum::<usize>(),
        1
    );
    if pause_at_end {
        let current = workflow
            .get_run(base::RunRequest {
                context: Some(call(session)),
                run_id: Some(run.to_string()),
                recipe_digest: vec![],
                site_config_digest: vec![],
            })
            .await
            .unwrap()
            .into_inner();
        let pause = base::RunRequest {
            context: Some(base::CallContext {
                request_key: Some(id().to_string()),
                expected_revision: Some(current.revision),
                ..call(session)
            }),
            run_id: Some(run.to_string()),
            recipe_digest: vec![],
            site_config_digest: vec![],
        };
        let paused = workflow
            .pause_run(pause.clone())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(paused.state, base::RunState::Paused as i32);
        assert_eq!(paused.revision, current.revision + 1);
        assert_eq!(
            workflow.pause_run(pause).await.unwrap().into_inner(),
            paused
        );
    }
}

fn graph_configuration(mut configuration: CellConfiguration) -> CellConfiguration {
    use rx_process_contract::*;
    let process = name("test/process");
    let source = SourceLocation {
        flow: name("main"),
        node: name("place"),
        instantiation: vec!["flow:main".into()],
    };
    let node = name(&format!(
        "node/{}",
        canonical::digest("RX-PROCESS-NODE-v1", &(&process, &source)).unwrap()
    ));
    configuration.steps[0].id = node.clone();
    let resolved = ResolvedProcess {
        schema: name("rx.resolved-process.v1"),
        package_digest: None,
        source_digest: Digest::from_bytes([65; 32]),
        process,
        root: CompiledNode {
            id: node,
            source,
            body: CompiledBody::Operation {
                binding: name("place"),
            },
        },
        bindings: BTreeMap::from([(
            name("place"),
            ActionBinding {
                host: configuration.steps[0].host.clone(),
                intent: configuration.steps[0].intent.clone(),
            },
        )]),
        conditions: BTreeMap::new(),
    };
    let bytes = canonical::bytes(&resolved).unwrap();
    configuration.recipe = ArtifactRef {
        sha256: rx_process_contract::frontier::resolved_digest(&resolved).unwrap(),
        schema_id: resolved.schema.clone(),
        size_bytes: Counter(bytes.len() as u64),
    };
    configuration.process = Some(Box::new(resolved));
    configuration
}

async fn wait_child(child: &mut std::process::Child) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("worker process timeout");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

struct LoseWorkerReply {
    handle: Handle<Application<SqliteRepository, TestClock, SimulationOnly>>,
    resolve: bool,
    submit: bool,
    pause: bool,
    pause_unavailable: bool,
    begin_part: bool,
    complete_part: bool,
    reconcile: bool,
    checkpoint: bool,
    expire: bool,
    timeout: bool,
    clock: Arc<std::sync::atomic::AtomicU64>,
    clock_file: std::path::PathBuf,
    first: std::sync::atomic::AtomicBool,
}
impl ApplicationPort for LoseWorkerReply {
    fn request(&self, command: Command) -> CallFuture<'_> {
        if self.pause_unavailable && matches!(&command, Command::PauseExecutorRun { .. }) {
            return Box::pin(async { Err(WriterError::Unavailable) });
        }

        let selected = (self.resolve
            && matches!(&command, Command::ExecutorResolveActivation { .. }))
            || (self.submit && matches!(&command, Command::ExecutorSubmit { .. }));
        let selected =
            selected || (self.pause && matches!(&command, Command::PauseExecutorRun { .. }));
        let selected = selected
            || (self.reconcile && matches!(&command, Command::RequestReconciliation { .. }));
        let selected =
            selected || (self.checkpoint && matches!(&command, Command::CommitCheckpoint { .. }));
        let is_checkpoint = matches!(&command, Command::CommitCheckpoint { .. });
        let alter_clock = is_checkpoint
            && (self.expire || self.timeout)
            && self.first.swap(false, std::sync::atomic::Ordering::SeqCst);
        let selected = selected
            || (self.begin_part && matches!(&command, Command::ExecutorBeginPart { .. }))
            || (self.complete_part && matches!(&command, Command::ExecutorCompletePart { .. }));
        let lose = selected && self.first.swap(false, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if alter_clock && self.expire {
                self.clock
                    .store(100_001_000, std::sync::atomic::Ordering::SeqCst);
                publish_clock(&self.clock_file, 100_001_000);
            }
            let reply = self.handle.call(command).await?;
            if alter_clock && self.timeout {
                self.clock.store(2000, std::sync::atomic::Ordering::SeqCst);
                publish_clock(&self.clock_file, 2000);
            }
            if lose {
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

fn publish_clock(path: &std::path::Path, ticks: u64) {
    let staged = path.with_extension("new");
    std::fs::write(&staged, serde_json::to_vec(&ticks.to_string()).unwrap()).unwrap();
    std::fs::rename(staged, path).unwrap();
}
