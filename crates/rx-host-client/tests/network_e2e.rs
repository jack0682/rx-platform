//! Run via tools/test_host_e2e.sh. Host is a separate process from the other repository.
#[path = "support/wire_executor.rs"]
mod wire_executor;
use rx_application::*;
use rx_domain::{
    budget::BudgetUnit, condition::Condition, intent::*, operation::Outcome, types::*,
};
use rx_host_client::{Hello, HostClient, TlsEndpoint};
use rx_storage::SqliteRepository;
use std::{
    collections::BTreeMap,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn artifact(n: u8, s: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([n; 32]),
        schema_id: name(s),
        size_bytes: Counter(1),
    }
}
#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "simulation/boottime".into(),
            ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
        }
    }
}
struct Catalog;
impl QualificationAuthority for Catalog {
    fn verify(&self, c: &CellConfiguration, e: &[ArtifactRef], d: &[Digest]) -> bool {
        c.environment == Environment::Simulation
            && e == [artifact(9, "rx.validation.simulation.v1")]
            && d == [
                c.definition.sha256,
                c.envelope.sha256,
                c.recipe.sha256,
                c.site_config_digest,
            ]
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn principal(id: &str, roles: &[Role]) -> Principal {
    Principal {
        id: name(id),
        client_namespace: name(&format!("client/{id}")),
        roles: roles.iter().copied().collect(),
        cells: [name("cell/sim")].into_iter().collect(),
        active: true,
    }
}
fn expiry() -> TimePoint {
    TimePoint {
        clock_id: "simulation/boottime".into(),
        ticks_ns: Counter(10_000_000_000),
    }
}

#[tokio::test]
#[ignore = "tools/test_host_e2e.sh builds the separate Host fixture and supplies its path"]
async fn application_outbox_reaches_mtls_host_and_t2_commits_the_result() {
    run_fixture(false, false).await;
}

#[tokio::test]
#[ignore = "tools/test_host_e2e.sh builds the separate Host fixture and supplies its path"]
async fn automatic_dispatcher_recovers_a_lost_authorize_reply_without_native_reexecution() {
    run_fixture(true, false).await;
}

#[tokio::test]
#[ignore = "tools/test_host_e2e.sh supplies the separate Host server"]
async fn remote_executor_submission_and_host_reply_loss_preserve_one_native_effect_per_slot() {
    run_fixture(true, true).await;
}
async fn run_fixture(automatic: bool, wire: bool) {
    use rcgen::*;
    use sha2::Digest as _;
    let executable = std::env::var("RX_HOST_SIM_SERVER").expect("Host executable path required");
    let directory = tempfile::tempdir().unwrap();
    let clock = TestClock(Arc::new(AtomicU64::new(1000)));
    let installation = id();
    let mut engine = Engine::open(
        SqliteRepository::open(directory.path().join("platform.db")).unwrap(),
        clock.clone(),
        Catalog,
        installation.clone(),
        principal(
            "operator",
            &[
                Role::AccountAdmin,
                Role::Engineer,
                Role::Verifier,
                Role::Operator,
                Role::Observer,
            ],
        ),
    )
    .unwrap();
    let login = engine
        .authenticated_session(&name("operator"), id(), expiry())
        .unwrap();
    let operator = Identity {
        principal: name("operator"),
        session: login.id,
        terminal: None,
    };
    engine
        .put_terminal(
            &operator,
            Terminal {
                id: name("panel"),
                certificate_digest: Digest::from_bytes([7; 32]),
                cells: [name("cell/sim")].into_iter().collect(),
                active: true,
            },
            None,
        )
        .unwrap();
    let bound = engine
        .authenticated_terminal_user_session(
            &operator.principal,
            id(),
            Counter(3_600_000_000_000),
            Digest::from_bytes([7; 32]),
        )
        .unwrap();
    let operator = Identity {
        principal: operator.principal,
        session: bound.id,
        terminal: Some((name("panel"), Digest::from_bytes([7; 32]))),
    };

    for p in [
        principal("executor", &[Role::Executor, Role::Observer]),
        principal("host/sim", &[Role::Host, Role::Observer]),
    ] {
        engine.put_principal(&operator, p, None).unwrap();
    }
    let executor = Identity {
        principal: name("executor"),
        session: if wire {
            id()
        } else {
            engine
                .authenticated_session(&name("executor"), id(), expiry())
                .unwrap()
                .id
        },
        terminal: None,
    };
    let ready = Condition::Eq {
        fact: name("ready"),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    };
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: name("sim/program"),
        profile_digest: Digest::from_bytes([21; 32]),
        site_config_digest: Digest::from_bytes([4; 32]),
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("sim/completed"),
        cancel_rule: name("sim/stop"),
        body: Body::Program(ProgramGoal {
            program: artifact(10, "rx.sim.program.v1"),
            parameter_set: artifact(11, "rx.sim.parameters.v1"),
        }),
    };
    let configuration = CellConfiguration {
        process: None,
        id: name("cell/sim"),
        environment: Environment::Simulation,
        definition: artifact(1, "rx.cell-definition.v1"),
        envelope: artifact(2, "rx.operating-envelope.v1"),
        recipe: artifact(3, "rx.resolved-recipe.v1"),
        site_config_digest: Digest::from_bytes([4; 32]),
        scopes: vec![name("scope/main")],
        hosts: vec![name("host/sim")],
        executor: name("executor"),
        maximum_budget: Counter(2),
        permit_ttl_ns: Counter(50_000_000),
        start_timeout_ns: Counter(1_000_000_000),
        start_conditions: vec![ready.clone()],
        maintained_conditions: vec![],
        steps: vec![StepBinding {
            id: name("step/run"),
            host: name("host/sim"),
            intent: intent.clone(),
            predecessors: vec![],
            conditions: vec![ready],
            condition_ids: vec![name("sim/ready")],
            condition_revision: Counter(1),
            handover_max_age_ns: Counter(20_000_000),
            completion: CompletionRule::Native {
                schema: name("rx.sim.completed.v1"),
                success: vec![Integer(0)],
                failure: vec![Integer(1)],
                postconditions: vec![],
            },
        }],
        fact_specs: vec![FactSpec {
            id: name("ready"),
            host: name("host/sim"),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            maximum_age_ns: Counter(1_000_000_000),
            maximum_uncertainty_ns: Counter(0),
        }],
    };
    let revision = engine
        .install_cell(&operator, configuration.clone())
        .unwrap();
    let qualification = engine
        .qualify(
            &operator,
            &configuration.id,
            revision,
            vec![artifact(9, "rx.validation.simulation.v1")],
            vec![
                configuration.definition.sha256,
                configuration.envelope.sha256,
                configuration.recipe.sha256,
                configuration.site_config_digest,
            ],
        )
        .unwrap();

    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let leaf = |label: &str, usage: ExtendedKeyUsagePurpose| {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec![label.into()]).unwrap();
        p.extended_key_usages = vec![usage];
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (p.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let (client, client_key) = leaf("platform", ExtendedKeyUsagePurpose::ClientAuth);
    let client_fp = Digest::from_bytes(sha2::Sha256::digest(client.der().as_ref()).into());
    for (filename, data) in [
        ("server.pem", server.pem()),
        ("server-key.pem", server_key.serialize_pem()),
        ("ca.pem", ca.pem()),
    ] {
        std::fs::write(directory.path().join(filename), data).unwrap();
    }
    let host_dir = directory.path().join("host");
    let release = Digest::from_bytes([99; 32]);
    let binding = serde_json::json!({"host":"host/sim","platform":installation.to_string(),"cell":"cell/sim",
        "definition":configuration.definition,"envelope":configuration.envelope,"qualification":qualification.id,
        "qualification_revision":"1","allowed_intents":[intent],"scope_ids":["scope/main"],"condition_ids":["sim/ready"],
        "environment":"SIMULATION","purposes":["PRODUCTION","SETUP"]});
    let config = serde_json::json!({"directory":host_dir,"bindings":[binding],"installation":installation,"release_digest":release,
        "client_fingerprint":client_fp,"server_certificate":directory.path().join("server.pem"),"server_key":directory.path().join("server-key.pem"),
        "client_ca":directory.path().join("ca.pem"),"clock_id":"simulation/boottime","ticks":1000});
    let config_path = directory.path().join("host-config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut process = Process(
        Command::new(executable)
            .arg(config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !host_dir.join("ready.json").exists() {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "Host exited during startup"
        );
        assert!(Instant::now() < deadline, "Host startup timeout");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(host_dir.join("ready.json")).unwrap()).unwrap();
    let endpoint = TlsEndpoint {
        uri: ready["address"].as_str().unwrap().into(),
        server_name: "localhost".into(),
        server_ca_pem: ca.pem().into_bytes(),
        client_certificate_pem: client.pem().into_bytes(),
        client_key_pem: client_key.serialize_pem().into_bytes(),
    };
    let hello = Hello {
        peer_id: name(engine.installation.id.as_str()),
        boot_id: engine.installation.runtime_boot.clone(),
        installation: engine.installation.id.clone(),
        store_generation: engine.installation.store_generation.clone(),
        release_digest: release,
        clock_id: clock.now().clock_id,
    };
    assert!(
        HostClient::connect_pinned(
            endpoint.clone(),
            name("host/sim"),
            hello.clone(),
            Digest::from_bytes([0; 32])
        )
        .await
        .is_err()
    );
    let client = HostClient::connect_pinned(
        endpoint,
        name("host/sim"),
        hello,
        Digest::from_bytes(sha2::Sha256::digest(server.der().as_ref()).into()),
    )
    .await
    .unwrap();
    let inspect = client
        .open_cell(&configuration, &clock.now().clock_id)
        .await
        .unwrap();
    let scopes: BTreeMap<_, _> = inspect
        .cell
        .as_ref()
        .unwrap()
        .scopes
        .iter()
        .map(|s| (name(&s.scope_id), Counter(s.epoch)))
        .collect();
    let snapshot = client
        .read_bootstrap(&configuration.id, vec![name("ready")])
        .await
        .unwrap();
    assert_eq!(
        snapshot.observations[0].generation.as_str(),
        ready["device_session"].as_str().unwrap()
    );
    let unsupported = client
        .read_bootstrap(&configuration.id, vec![name("unknown-source")])
        .await
        .unwrap();
    assert!(!unsupported.sources_available && unsupported.observations.is_empty());
    // The incoming credential adapter is simulated here; the real publisher handshake has its own TLS test.
    let session = engine
        .open_evidence_producer(
            &name("host/sim"),
            snapshot.host_boot.clone(),
            snapshot.evidence_journal.clone(),
            Digest::from_bytes([87; 32]),
        )
        .unwrap();
    let host_identity = Identity {
        principal: name("host/sim"),
        session: session.id,
        terminal: None,
    };
    engine
        .negotiate_evidence_cell(&host_identity, configuration.definition.sha256)
        .unwrap();
    let initial_source = snapshot.observations[0].clone();
    if !automatic {
        let fence = client
            .fence(&id(), &configuration.id, Counter(1), &scopes, &[])
            .await
            .unwrap();
        let (grant, host_boot) = client
            .acquire_grant(
                &id(),
                vec![name("sim/controller")],
                Counter(1),
                Counter(1000),
                clock.now(),
            )
            .await
            .unwrap();
        let device_session = initial_source.generation.clone();
        engine
            .register_host(
                &host_identity,
                HostRegistration {
                    id: name("host/sim"),
                    session: host_identity.session.clone(),
                    boot_id: host_boot,
                    delivery_journal: Id::new(fence.delivery_journal_id).unwrap(),
                    cell: configuration.id.clone(),
                    epoch: Counter(1),
                    scopes: scopes.clone(),
                    source_sessions: [(name("ready"), device_session.clone())]
                        .into_iter()
                        .collect(),
                    grant,
                },
            )
            .unwrap();
        // Explicit scenario precondition, not inferred from TLS connectivity.
        engine
            .report_fact(
                &host_identity,
                FactRecord {
                    cell: configuration.id.clone(),
                    id: name("ready"),
                    source_host: name("host/sim"),
                    source_generation: device_session,
                    schema: name("boolean/v1"),
                    unit: name("unitless"),
                    acquired_at: clock.now(),
                    maximum_age_ns: Counter(1_000_000_000),
                    acquisition_uncertainty_ns: Counter(0),
                    quality_good: true,
                    origin_age_bounded: true,
                    disputed: false,
                    value: TypedValue::Boolean(true),
                    evidence_id: id(),
                },
            )
            .unwrap();
    }
    let wire = if wire {
        let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
        let (executor, executor_key) = leaf("executor", ExtendedKeyUsagePurpose::ClientAuth);
        Some(wire_executor::TlsFixture {
            server: rx_api::grpc::TlsMaterial {
                server_certificate_pem: server.pem().into_bytes(),
                server_key_pem: server_key.serialize_pem().into_bytes(),
                client_ca_pem: ca.pem().into_bytes(),
            },
            ca: ca.pem().into_bytes(),
            certificate: executor.pem().into_bytes(),
            key: executor_key.serialize_pem().into_bytes(),
            fingerprint: Digest::from_bytes(sha2::Sha256::digest(executor.der().as_ref()).into()),
            release,
        })
    } else {
        None
    };
    if automatic {
        run_automatic(AutomaticFixture {
            wire,
            engine,
            client,
            configuration,
            operator,
            executor,
            host_identity,
            host_dir,
        })
        .await;
        return;
    }
    let cell_revision = engine.inspect_cell(&operator, &configuration.id).unwrap().0;
    let run = engine
        .create_run(
            &operator,
            id().as_str(),
            CreateRun {
                cell: configuration.id.clone(),
                recipe_digest: configuration.recipe.sha256,
                site_config_digest: configuration.site_config_digest,
                expected_cell: cell_revision,
            },
        )
        .unwrap();
    let attempt = engine
        .start_run(
            &operator,
            id().as_str(),
            StartRun {
                run: run.id.clone(),
                envelope_digest: configuration.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: BudgetUnit::PartAttempt,
                budget_limit: Counter(2),
                expected_cell: cell_revision,
                expected_run: Counter(1),
            },
        )
        .unwrap();
    let arm = engine
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(d.payload, Delivery::Arm { .. }))
        .unwrap();
    engine.begin_delivery(&arm.id).unwrap();
    let ack = client
        .arm(&arm.id, &attempt.id, &configuration.id, Counter(1), &scopes)
        .await
        .unwrap();
    engine
        .finish_arm_delivery(&host_identity, &arm.id, ack)
        .unwrap();
    let cell_revision = engine.inspect_cell(&operator, &configuration.id).unwrap().0;
    for ordinal in 1..=2 {
        let current = engine.inspect_run(&executor, &run.id).unwrap().1;
        let part = engine
            .begin_part(
                &executor,
                id().as_str(),
                &run.id,
                current.budget.as_ref().unwrap().revision(),
            )
            .unwrap();
        let run_revision = engine.inspect_run(&executor, &run.id).unwrap().0;
        let activation = engine
            .resolve_activation(
                &executor,
                &run.id,
                &name("step/run"),
                part.ordinal,
                run_revision,
            )
            .unwrap();
        let run_revision = engine.inspect_run(&executor, &run.id).unwrap().0;
        let work = engine
            .submit(
                &executor,
                id().as_str(),
                SubmitWork {
                    run: run.id.clone(),
                    activation: activation.id,
                    part: Some(part.id.clone()),
                    slot: name("main"),
                    intent: configuration.steps[0].intent.clone(),
                    expected_cell: cell_revision,
                    expected_run: run_revision,
                },
            )
            .unwrap();
        let permit = engine.inspect_permit(&operator, &work.permit).unwrap();
        engine.begin_delivery(work.operation.id()).unwrap();
        let receipt = client
            .prepare(work.operation.id(), &work, &permit)
            .await
            .unwrap();
        engine
            .record_host_receipt(&host_identity, work.operation.id(), receipt)
            .unwrap();
        let dispatch = engine
            .pending_deliveries(128)
            .unwrap()
            .into_iter()
            .find(|d| matches!(d.payload, Delivery::Authorize { .. }))
            .unwrap();
        let work = engine.inspect_work(&operator, work.operation.id()).unwrap();
        engine.begin_delivery(&dispatch.id).unwrap();
        let receipt = client
            .authorize(&dispatch.id, &work, &permit)
            .await
            .unwrap();
        engine
            .record_host_receipt(&host_identity, &dispatch.id, receipt)
            .unwrap();
        assert_eq!(
            engine
                .inspect_work(&operator, work.operation.id())
                .unwrap()
                .operation
                .outcome(),
            Outcome::None
        );
        let evidence = client.reconcile(work.operation.id()).await.unwrap();
        let ack = engine
            .ingest_evidence(&host_identity, evidence.clone())
            .unwrap();
        assert_eq!(ack.through, Counter(ordinal));
        engine.ingest_evidence(&host_identity, evidence).unwrap();
        assert_eq!(
            engine
                .inspect_work(&operator, work.operation.id())
                .unwrap()
                .operation
                .outcome(),
            Outcome::Succeeded
        );
        let done = engine.inspect_work(&operator, work.operation.id()).unwrap();
        let observations = client.handover(work.operation.id()).await.unwrap();
        let released = engine
            .release_resources(
                &host_identity,
                id().as_str(),
                ReleaseResources {
                    operation: work.operation.id().clone(),
                    expected_operation: done.operation.revision(),
                    expected_cell: cell_revision,
                    observations,
                },
            )
            .unwrap();
        assert_eq!(
            released.operation.disposition(),
            rx_domain::operation::Disposition::Released
        );
        let revision = engine.inspect_run(&executor, &run.id).unwrap().0;
        engine
            .complete_part(&executor, id().as_str(), &part.id, revision)
            .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(host_dir.join("device/effects.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert_eq!(
        engine.inspect_run(&operator, &run.id).unwrap().1.state,
        RunState::Completed
    );
}

struct AutomaticFixture {
    wire: Option<wire_executor::TlsFixture>,
    engine: Engine<SqliteRepository, TestClock, Catalog>,
    client: HostClient,
    configuration: CellConfiguration,
    operator: Identity,
    executor: Identity,
    host_identity: Identity,
    host_dir: std::path::PathBuf,
}
struct LostAuthorizeReply {
    client: HostClient,
    first: std::sync::atomic::AtomicBool,
    calls: std::sync::atomic::AtomicUsize,
    handover_calls: std::sync::atomic::AtomicUsize,
    bad_support_once: std::sync::atomic::AtomicBool,
}
#[tonic::async_trait]
impl rx_host_client::delivery::DeliveryTransport for LostAuthorizeReply {
    fn host_id(&self) -> &Name {
        &self.client.host_id
    }
    async fn arm(
        &self,
        key: &Id,
        attempt: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
    ) -> Result<ArmAcknowledgment, tonic::Status> {
        self.client.arm(key, attempt, cell, epoch, scopes).await
    }
    async fn fence(
        &self,
        key: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        blocks: &[Id],
    ) -> Result<rx_protocol::cell::FenceReceipt, tonic::Status> {
        self.client.fence(key, cell, epoch, scopes, blocks).await
    }
    async fn prepare(
        &self,
        key: &Id,
        work: &Work,
        permit: &Permit,
    ) -> Result<HostReceipt, tonic::Status> {
        self.client.prepare(key, work, permit).await
    }
    async fn authorize(
        &self,
        key: &Id,
        work: &Work,
        permit: &Permit,
    ) -> Result<HostReceipt, tonic::Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let receipt = self.client.authorize(key, work, permit).await?;
        if self.first.swap(false, Ordering::SeqCst) {
            Err(tonic::Status::unavailable(
                "injected reply loss after actual Host authorization",
            ))
        } else {
            Ok(receipt)
        }
    }
    async fn receipt(&self, operation: &Id) -> Result<HostReceipt, tonic::Status> {
        self.client.receipt(operation).await
    }
    async fn reconcile(&self, operation: &Id) -> Result<EvidenceBatch, tonic::Status> {
        self.client.reconcile(operation).await
    }
    async fn handover(&self, operation: &Id) -> Result<Vec<HandoverObservation>, tonic::Status> {
        self.handover_calls.fetch_add(1, Ordering::SeqCst);
        let mut observations = self.client.handover(operation).await?;
        if self.bad_support_once.swap(false, Ordering::SeqCst) {
            observations
                .iter_mut()
                .find(|v| v.source.as_str().ends_with("/support"))
                .unwrap()
                .value = false;
        }
        Ok(observations)
    }
}
async fn run_automatic(mut f: AutomaticFixture) {
    use rx_host_client::delivery::Dispatcher;
    use rx_runtime::{
        application::{Application, Command as Call, Handle, Reply},
        writer::Writer,
    };
    let writer = Writer::start(move || Ok(Application::new(f.engine)))
        .await
        .unwrap();
    let handle = Handle::new(writer);
    let mut connected = rx_host_client::connection::ConnectedHost::from_client(
        Arc::new(handle.clone()),
        f.client.clone(),
        &f.configuration,
        Counter(1000),
    )
    .await
    .unwrap();
    assert_eq!(connected.identity().session, f.host_identity.session);
    connected.renew().await.unwrap();
    let observed = connected.observe().await.unwrap();
    assert_eq!(observed.entries.len(), 1);
    assert_eq!(
        observed.entries[0].disposition,
        rx_application::observation::Disposition::Current
    );
    let reused = rx_host_client::connection::ConnectedHost::from_client(
        Arc::new(handle.clone()),
        f.client.clone(),
        &f.configuration,
        Counter(1000),
    )
    .await
    .unwrap();
    assert_eq!(
        reused.registration.grant.id,
        connected.registration.grant.id
    );
    let mut remote = if let Some(tls) = f.wire.take() {
        Some(
            wire_executor::RemoteExecutor::connect(handle.clone(), f.configuration.clone(), tls)
                .await,
        )
    } else {
        None
    };
    if let Some(client) = &remote {
        f.executor = client.identity.clone();
    }

    let transport = Arc::new(LostAuthorizeReply {
        client: f.client.clone(),
        first: std::sync::atomic::AtomicBool::new(true),
        calls: std::sync::atomic::AtomicUsize::new(0),
        handover_calls: std::sync::atomic::AtomicUsize::new(0),
        bad_support_once: std::sync::atomic::AtomicBool::new(true),
    });
    let dispatcher = Dispatcher::with_transport(
        Arc::new(handle.clone()),
        transport.clone(),
        f.host_identity.clone(),
    )
    .unwrap();
    let report = dispatcher.subscribe();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let sender = tokio::spawn(dispatcher.run(stopped.clone()));
    let reader = rx_host_client::observation::ObservationReader::new(
        Arc::new(handle.clone()),
        f.client.clone(),
        connected.plan.id.clone(),
        vec![name("ready")],
        Duration::from_millis(25),
    )
    .unwrap();
    let mut readings = reader.subscribe();
    let observer = tokio::spawn(reader.run(stopped));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let rx_host_client::observation::ObservationStatus::Received(receipt) =
                &*readings.borrow()
                && receipt.entries[0].evidence != observed.entries[0].evidence
            {
                break;
            }
            readings.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let Reply::Cell(revision, _) = handle
        .call(Call::InspectCell {
            identity: f.operator.clone(),
            cell: f.configuration.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let Reply::Run(run) = handle
        .call(Call::CreateRun {
            identity: f.operator.clone(),
            key: id(),
            command: CreateRun {
                cell: f.configuration.id.clone(),
                recipe_digest: f.configuration.recipe.sha256,
                site_config_digest: f.configuration.site_config_digest,
                expected_cell: revision,
            },
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    handle
        .call(Call::StartRun {
            identity: f.operator.clone(),
            key: id(),
            command: StartRun {
                run: run.id.clone(),
                envelope_digest: f.configuration.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: BudgetUnit::PartAttempt,
                budget_limit: Counter(2),
                expected_cell: revision,
                expected_run: Counter(1),
            },
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let Reply::VersionedRun(_, current) = handle
            .call(Call::InspectRun {
                identity: f.executor.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("run")
        };
        if current.state == RunState::Executing {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "automatic Arm timeout: {:?}",
            *report.borrow()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let Reply::Cell(revision, _) = handle
        .call(Call::InspectCell {
            identity: f.operator.clone(),
            cell: f.configuration.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("current cell")
    };
    for _ in 0..2 {
        let Reply::VersionedRun(_, current) = handle
            .call(Call::InspectRun {
                identity: f.executor.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("run")
        };
        let mandate = current.mandate.clone().unwrap();
        let part = if let Some(client) = &mut remote {
            client.begin_part(&current, revision).await
        } else {
            let Reply::Part(part) = handle
                .call(Call::BeginPart {
                    identity: f.executor.clone(),
                    key: id(),
                    run: run.id.clone(),
                    expected_budget: current.budget.unwrap().revision(),
                })
                .await
                .unwrap()
            else {
                panic!("part")
            };
            part
        };
        let Reply::VersionedRun(run_revision, _) = handle
            .call(Call::InspectRun {
                identity: f.executor.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("run")
        };
        let activation_id = if let Some(client) = &mut remote {
            client.activation(&run.id, part.ordinal, run_revision).await
        } else {
            let Reply::Activation(activation) = handle
                .call(Call::ResolveActivation {
                    identity: f.executor.clone(),
                    run: run.id.clone(),
                    node: name("step/run"),
                    visit: part.ordinal,
                    expected_run: run_revision,
                })
                .await
                .unwrap()
            else {
                panic!("activation")
            };
            activation.id
        };
        let Reply::VersionedRun(run_revision, _) = handle
            .call(Call::InspectRun {
                identity: f.executor.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("run")
        };
        let command = SubmitWork {
            run: run.id.clone(),
            activation: activation_id,
            part: Some(part.id.clone()),
            slot: name("main"),
            intent: f.configuration.steps[0].intent.clone(),
            expected_cell: revision,
            expected_run: run_revision,
        };
        let work = if let Some(client) = &mut remote {
            let operation = client.submit(&command, &mandate).await;
            let Reply::Work(work) = handle
                .call(Call::InspectWork {
                    identity: f.operator.clone(),
                    operation,
                })
                .await
                .unwrap()
            else {
                panic!("work")
            };
            work
        } else {
            let Reply::Work(work) = handle
                .call(Call::SubmitWork {
                    identity: f.executor.clone(),
                    key: id(),
                    command: Box::new(command),
                })
                .await
                .unwrap()
            else {
                panic!("work")
            };
            work
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let done = loop {
            let Reply::Work(mut current) = handle
                .call(Call::InspectWork {
                    identity: f.operator.clone(),
                    operation: work.operation.id().clone(),
                })
                .await
                .unwrap()
            else {
                panic!("work")
            };
            let succeeded = if let Some(client) = &mut remote {
                let view = client.get(work.operation.id()).await;
                assert_eq!(view.operation_id, work.operation.id().to_string());
                if view.outcome == rx_protocol::base::Outcome::Succeeded as i32 {
                    let Reply::Work(latest) = handle
                        .call(Call::InspectWork {
                            identity: f.operator.clone(),
                            operation: work.operation.id().clone(),
                        })
                        .await
                        .unwrap()
                    else {
                        panic!("latest work")
                    };
                    current = latest;
                    assert_eq!(current.operation.outcome(), Outcome::Succeeded);
                    assert_eq!(view.integrity, rx_protocol::base::Integrity::Valid as i32);
                    assert!(
                        matches!(
                            rx_protocol::base::Disposition::try_from(view.disposition),
                            Ok(rx_protocol::base::Disposition::Held
                                | rx_protocol::base::Disposition::Quarantined)
                        ),
                        "completion does not itself confirm resource handover"
                    );
                    true
                } else {
                    false
                }
            } else {
                current.operation.outcome() == Outcome::Succeeded
            };
            if succeeded {
                break current;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "automatic result timeout: {:?}",
                *report.borrow()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        if let Some(client) = &mut remote {
            let first = client.reconcile(work.operation.id()).await;
            assert_ne!(
                first.disposition,
                rx_protocol::base::Disposition::Released as i32
            );
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let view = client.get(work.operation.id()).await;
                if view.disposition == rx_protocol::base::Disposition::Released as i32 {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "automatic handover timed out: {:?}",
                    *report.borrow()
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        } else {
            let observations = f.client.handover(work.operation.id()).await.unwrap();
            handle
                .call(Call::ReleaseResources {
                    identity: f.host_identity.clone(),
                    key: id(),
                    command: ReleaseResources {
                        operation: work.operation.id().clone(),
                        expected_operation: done.operation.revision(),
                        expected_cell: revision,
                        observations,
                    },
                })
                .await
                .unwrap();
        }
        let Reply::VersionedRun(run_revision, _) = handle
            .call(Call::InspectRun {
                identity: f.executor.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("run")
        };
        handle
            .call(Call::CompletePart {
                identity: f.executor.clone(),
                key: id(),
                part: part.id,
                expected_run: run_revision,
            })
            .await
            .unwrap();
    }
    if remote.is_some() {
        assert!(
            transport.handover_calls.load(Ordering::SeqCst) >= 3,
            "false support observation released a resource"
        );
    }
    assert_eq!(
        transport.calls.load(Ordering::SeqCst),
        2,
        "authorization must not be resent after SEND_ENTERED"
    );
    assert!(
        report.borrow().attention >= 1,
        "reply-loss branch must be exercised"
    );
    assert_eq!(
        std::fs::read_to_string(f.host_dir.join("device/effects.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let Reply::VersionedRun(_, done) = handle
        .call(Call::InspectRun {
            identity: f.operator.clone(),
            run: run.id,
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    assert_eq!(done.state, RunState::Completed);
    handle
        .call(Call::Hold {
            identity: f.operator,
            key: id(),
            cell: f.configuration.id.clone(),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = f.client.inspect_cell(&f.configuration.id).await.unwrap();
        if state.cell.as_ref().is_some_and(|c| c.cell_epoch == 2) && !state.block_ids.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "automatic fence timeout: {:?}",
            *report.borrow()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        report.borrow().last_pass_at.is_some(),
        "dispatcher must report an actual completed loop pass"
    );
    stop.send(true).unwrap();
    sender.await.unwrap().unwrap();
    observer.await.unwrap().unwrap();
    assert!(matches!(
        *readings.borrow(),
        rx_host_client::observation::ObservationStatus::Stopped
    ));
    if let Some(client) = remote {
        client.shutdown().await;
    }
    handle.close();
    handle.closed().await;
}
