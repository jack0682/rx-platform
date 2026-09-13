//! Real terminal TLS/writer authentication with explicit BFF response fixtures.
//! The intercepted investigation records below are routing data, never a core T5 acceptance.
mod support;
use reqwest::{Client, StatusCode};
use rx_api::{
    auth::{Credentials, LocalAccount, password_hash},
    terminal_https::{HttpsPolicy, TerminalHttps, TlsMaterial},
};
use rx_application::{investigation as data, *};
use rx_domain::{
    fault::Rejection,
    operation::{Conclusion, Operation, Outcome},
    types::*,
};
use rx_ports::StoreError;
use rx_runtime::{
    application::{Application, ApplicationPort, CallFuture, Command, Handle, Reply},
    writer::{Status, Writer, WriterError},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn at() -> TimePoint {
    TimePoint {
        clock_id: "investigation-http-test".into(),
        ticks_ns: Counter(1000),
    }
}
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        at()
    }
}
struct Deny;
impl QualificationAuthority for Deny {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type Runtime = Handle<Application<rx_storage::SqliteRepository, TestClock, Deny>>;
fn principal(label: &str, roles: &[Role]) -> Principal {
    Principal {
        id: name(label),
        client_namespace: name(&format!("test/{label}")),
        roles: roles.iter().copied().collect(),
        cells: [name("cell/a")].into(),
        active: true,
    }
}

#[derive(Clone)]
struct Records {
    context: data::Context,
    attestation: data::Attestation,
    receipt: data::Receipt,
}
fn records() -> Records {
    let cfg = support::configuration("cell/a");
    let step = cfg.steps[0].clone();
    let digest = Digest::from_bytes([9; 32]);
    let operation_id = id();
    let permit_id = id();
    let mut operation = Operation::admitted(operation_id.clone(), step.intent.digest().unwrap());
    operation.sent().unwrap();
    operation.lose_continuity().unwrap();
    let work = Work {
        operation,
        intent: step.intent.clone(),
        cell: cfg.id.clone(),
        run: id(),
        part: None,
        activation: id(),
        slot: name("main"),
        host: step.host.clone(),
        permit: permit_id.clone(),
        invocation: Some(id()),
        completion: step.completion,
        host_journal: id(),
        handover_max_age_ns: Counter(500_000_000),
    };
    let permit = Permit {
        id: permit_id,
        operation: operation_id.clone(),
        intent_digest: work.intent.digest().unwrap(),
        cell: cfg.id.clone(),
        mandate: id(),
        epoch: Counter(1),
        scopes: [(name("zone/shared"), Counter(1))].into(),
        host: step.host,
        host_boot: id(),
        expires_at: at(),
        state: PermitState::Voided,
        grant: Grant {
            id: id(),
            fence: Counter(1),
            resources: work.intent.resource_set.clone(),
            owner: name("platform/test"),
            valid_until: at(),
            ttl_ms: Counter(1000),
        },
        qualification: id(),
        evidence_ids: vec![],
        qualification_revision: Counter(1),
        envelope_digest: cfg.envelope.sha256,
        purpose: Purpose::Production,
        issued_at: at(),
        condition_ids: vec![name("ready")],
        condition_revision: Counter(1),
    };
    let procedure = data::Procedure {
        schema: name(rx_process_contract::investigation::SCHEMA),
        id: name("simulation/unknown"),
        revision: Counter(1),
        title: "Routing fixture only".into(),
        instructions: vec!["Retain the unknown result and quarantine.".into()],
        cell: cfg.id.clone(),
        definition: cfg.definition.sha256,
        environment: name("SIMULATION"),
        profiles: [work.intent.profile_digest].into(),
        action: data::Action::AbandonInvestigation,
    };
    let reference = procedure.reference().unwrap();
    let evidence = id();
    let context = data::Context {
        installation: id(),
        store_generation: id(),
        runtime_boot: id(),
        work_revision: Counter(3),
        work: work.clone(),
        permit,
        cells: [(
            cfg.id.clone(),
            data::CellCut {
                revision: Counter(2),
                epoch: Counter(2),
                scopes: [(name("zone/shared"), Counter(2))].into(),
                configuration_digest: digest,
                definition: cfg.definition.sha256,
                environment: name("SIMULATION"),
                blocks: vec![],
            },
        )]
        .into(),
        evidence: Default::default(),
        policy: data::PolicyGeneration {
            id: id(),
            runtime_boot: id(),
            digest,
            file_digest: digest,
        },
        procedures: vec![reference.clone()],
        operation_authorized: false,
        resource_release_authorized: false,
    };
    let actor = data::Actor {
        principal: name("lead"),
        session: id(),
        terminal: Some((name("historical-panel"), digest)),
    };
    let request = data::AttestSubmit {
        id: id(),
        operation: operation_id.clone(),
        expected_operation_revision: work.operation.revision(),
        context_digest: context.digest().unwrap(),
        procedure_digest: reference.sha256,
        evidence_ids: vec![evidence],
        assertion: data::Assertion::ResultRemainsUnknown,
        note: "Routing response fixture; no field assessment.".into(),
        occurred_at: "2026-09-13T00:00:00Z".into(),
    };
    let attestation = data::Attestation {
        schema: name(data::ATTESTATION_SCHEMA),
        id: request.id.clone(),
        request,
        actor: actor.clone(),
        recorded_at: at(),
        context: context.clone(),
        procedure,
        procedure_reference: reference,
        signature: rx_package::SignatureEnvelope {
            key: name("routing-only"),
            signature: "00".repeat(64),
        },
        operation_authorized: false,
        resource_release_authorized: false,
    };
    let mut after = work.clone();
    after
        .operation
        .conclude(Conclusion {
            outcome: Outcome::Unresolved,
            evidence_ids: vec![attestation.id.clone()],
        })
        .unwrap();
    let receipt = data::Receipt {
        schema: name(data::DISPOSITION_SCHEMA),
        id: id(),
        request: data::RecordDisposition {
            operation: operation_id,
            expected_revision: work.operation.revision(),
            evidence_ids: vec![attestation.id.clone()],
            procedure_digest: attestation.procedure_reference.sha256,
            disposition: data::Disposition::Quarantined,
            reason: data::Reason::UnknownOutcome,
        },
        actor,
        recorded_at: at(),
        attestation: attestation.clone(),
        attestation_reference: attestation.reference().unwrap(),
        before: work,
        after,
        operation_authorized: false,
        resource_release_authorized: false,
        resources_released: false,
    };
    Records {
        context,
        attestation,
        receipt,
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Reject,
    Cached,
    InvalidAuthority,
}
struct RoutingPort {
    runtime: Runtime,
    records: Records,
    calls: Mutex<Vec<Value>>,
    mode: Mutex<Mode>,
}
impl RoutingPort {
    fn record(&self, method: &str, identity: &Identity, key: Option<&Id>, input: Value) {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"method":method,"principal":identity.principal,
            "session":identity.session,"terminal":identity.terminal,"key":key,"input":input}));
    }
    fn attestation_view(&self, invalid: bool) -> data::AttestationView {
        data::AttestationView {
            attestation: self.records.attestation.clone(),
            current: false,
            operation_authorized: invalid,
            resource_release_authorized: false,
        }
    }
    fn receipt_view(&self, invalid: bool) -> data::ReceiptView {
        data::ReceiptView {
            receipt: self.records.receipt.clone(),
            current: false,
            operation_authorized: false,
            resource_release_authorized: invalid,
        }
    }
}
impl ApplicationPort for RoutingPort {
    fn request(&self, command: Command) -> CallFuture<'_> {
        let mode = *self.mode.lock().unwrap();
        let reject = || {
            Err(WriterError::Rejected(StoreError::Rejected(
                Rejection::NotFound,
            )))
        };
        let result = match command {
            Command::InvestigationContext {
                identity,
                operation,
            } => {
                self.record("context", &identity, None, json!({"operation":operation}));
                if mode == Mode::Reject {
                    reject()
                } else {
                    let mut value = self.records.context.clone();
                    value.operation_authorized = mode == Mode::InvalidAuthority;
                    Ok(Reply::InvestigationContext(Box::new(value)))
                }
            }
            Command::PrepareInvestigationAttestation {
                identity,
                key,
                input,
            } => {
                self.record(
                    "attest-preflight",
                    &identity,
                    Some(&key),
                    serde_json::to_value(input).unwrap(),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationPreflight(data::Preflight::Recorded(
                        Box::new(self.records.attestation.clone()),
                    )))
                }
            }
            Command::GetInvestigationAttestation {
                identity,
                operation,
                id,
            } => {
                self.record(
                    "attest-get",
                    &identity,
                    None,
                    json!({"operation":operation,"id":id}),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationAttestationView(Box::new(
                        self.attestation_view(mode == Mode::InvalidAuthority),
                    )))
                }
            }
            Command::ListInvestigationAttestations {
                identity,
                operation,
                after,
                limit,
            } => {
                self.record(
                    "attest-list",
                    &identity,
                    None,
                    json!({"operation":operation,"after":after,"limit":limit}),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationAttestationPage(data::AttestationPage {
                        items: vec![self.attestation_view(mode == Mode::InvalidAuthority)],
                        next: None,
                    }))
                }
            }
            Command::PrepareInvestigationDisposition {
                identity,
                key,
                input,
            } => {
                self.record(
                    "disposition-preflight",
                    &identity,
                    Some(&key),
                    serde_json::to_value(input).unwrap(),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationDispositionPreflight(
                        data::DispositionPreflight::Recorded(Box::new(
                            self.records.receipt.clone(),
                        )),
                    ))
                }
            }
            Command::GetInvestigationDisposition {
                identity,
                operation,
                id,
            } => {
                self.record(
                    "disposition-get",
                    &identity,
                    None,
                    json!({"operation":operation,"id":id}),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationReceiptView(Box::new(
                        self.receipt_view(mode == Mode::InvalidAuthority),
                    )))
                }
            }
            Command::ListInvestigationDispositions {
                identity,
                operation,
                after,
                limit,
            } => {
                self.record(
                    "disposition-list",
                    &identity,
                    None,
                    json!({"operation":operation,"after":after,"limit":limit}),
                );
                if mode == Mode::Reject {
                    reject()
                } else {
                    Ok(Reply::InvestigationReceiptPage(data::ReceiptPage {
                        items: vec![self.receipt_view(mode == Mode::InvalidAuthority)],
                        next: None,
                    }))
                }
            }
            command => return self.runtime.request(command),
        };
        Box::pin(async move { result })
    }
    fn status(&self) -> Status {
        self.runtime.status()
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: Runtime,
    root: Identity,
    port: Arc<RoutingPort>,
    client: Client,
    no_certificate: Client,
    origin: String,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
async fn fixture() -> Fixture {
    use rcgen::*;
    use sha2::Digest as _;
    let mut ca = CertificateParams::new(vec![]).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(ca, KeyPair::generate().unwrap()).unwrap();
    let leaf = |server: bool| {
        let key = KeyPair::generate().unwrap();
        let mut parameters = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        parameters.extended_key_usages = vec![if server {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (parameters.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf(true);
    let (certificate, key) = leaf(false);
    let fingerprint = Digest::from_bytes(sha2::Sha256::digest(certificate.der().as_ref()).into());
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("platform.db");
    let root_session = id();
    let session = root_session.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            rx_storage::SqliteRepository::open(database)?,
            TestClock,
            Deny,
            id(),
            principal(
                "root",
                &[Role::AccountAdmin, Role::Engineer, Role::Observer],
            ),
        )?;
        engine.authenticated_session(
            &name("root"),
            session.clone(),
            TimePoint {
                clock_id: at().clock_id,
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let root = Identity {
            principal: name("root"),
            session,
            terminal: None,
        };
        for (label, role) in [
            ("lead", Role::RecoveryLead),
            ("operator", Role::Operator),
            ("engineer", Role::Engineer),
        ] {
            engine.put_principal(&root, principal(label, &[role, Role::Observer]), None)?;
        }
        engine.put_terminal(
            &root,
            Terminal {
                id: name("panel/a"),
                certificate_digest: fingerprint,
                cells: [name("cell/a")].into(),
                active: true,
            },
            None,
        )?;
        engine.install_cell(&root, support::configuration("cell/a"))?;
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let runtime = Handle::new(writer);
    let port = Arc::new(RoutingPort {
        runtime: runtime.clone(),
        records: records(),
        calls: Mutex::new(vec![]),
        mode: Mutex::new(Mode::Reject),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    static HASH: OnceLock<String> = OnceLock::new();
    let hash = HASH.get_or_init(|| password_hash("investigation-route-test-only").unwrap());
    let credentials = Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: ["lead", "operator", "engineer"]
            .into_iter()
            .map(|p| LocalAccount {
                principal: name(p),
                password_hash: hash.clone(),
            })
            .collect(),
    };
    let https = TerminalHttps::new_with_investigation(
        port.clone(),
        credentials,
        HttpsPolicy::new(&origin).unwrap(),
        TlsMaterial {
            server_certificate_pem: server.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            terminal_ca_pem: ca.pem().into_bytes(),
        },
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(https.serve(listener, async {
        let _ = stopped.await;
    }));
    let builder = || {
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(ca.pem().as_bytes()).unwrap())
    };
    let client = builder()
        .identity(
            reqwest::Identity::from_pem(
                format!("{}{}", certificate.pem(), key.serialize_pem()).as_bytes(),
            )
            .unwrap(),
        )
        .build()
        .unwrap();
    Fixture {
        _directory: directory,
        runtime,
        root: Identity {
            principal: name("root"),
            session: root_session,
            terminal: None,
        },
        port,
        client,
        no_certificate: builder().build().unwrap(),
        origin,
        stop,
        task,
    }
}
impl Fixture {
    fn attestation_input(&self, key: &Id) -> Value {
        json!({"request_key":key,"command":self.port.records.attestation.request})
    }
    fn disposition_input(&self, key: &Id) -> Value {
        json!({"request_key":key,"command":self.port.records.receipt.request})
    }
    async fn login(&self, who: &str) -> String {
        let reply = self
            .client
            .post(format!("{}/api/v1/session", self.origin))
            .header("Origin", &self.origin)
            .header("X-RX-Client", "browser-v1")
            .json(&json!({"principal":who,"password":"investigation-route-test-only"}))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        reply
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into()
    }
    async fn send(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.origin));
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie);
        }
        if let Some(body) = body {
            request = request
                .header("Origin", &self.origin)
                .header("X-RX-Client", "browser-v1")
                .json(&body);
        }
        let reply = request.send().await.unwrap();
        (reply.status(), reply.json().await.unwrap())
    }
    async fn finish(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap().unwrap();
        self.runtime.close();
        self.runtime.closed().await;
    }
}

#[tokio::test]
async fn authentication_and_preflight_precede_missing_worker_or_cached_response() {
    let f = fixture().await;
    let operation = f.port.records.context.work.operation.id();
    let path = format!("/api/v1/investigation-context?operation={operation}");
    assert_eq!(
        f.send("GET", &path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/investigation-attestations",
            None,
            Some(f.attestation_input(&id()))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/recovery-dispositions",
            None,
            Some(f.disposition_input(&id()))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        f.no_certificate
            .get(format!("{}{path}", f.origin))
            .send()
            .await
            .is_err()
    );
    for who in ["operator", "engineer"] {
        let cookie = f.login(who).await;
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/investigation-attestations",
                Some(&cookie),
                Some(f.attestation_input(&id()))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(f.port.calls.lock().unwrap().is_empty());
    let cookie = f.login("lead").await;
    // Rejected core preflight wins over unavailable worker; no fake ticket is manufactured.
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/investigation-attestations",
            Some(&cookie),
            Some(f.attestation_input(&id()))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.port.calls.lock().unwrap()[0]["method"],
        "attest-preflight"
    );
    f.finish().await;
}

#[tokio::test]
async fn strict_body_and_pagination_reject_claimed_authority_and_keep_csrf_checks() {
    let f = fixture().await;
    let cookie = f.login("lead").await;
    let operation = f.port.records.context.work.operation.id();
    for field in [
        "actor",
        "session",
        "terminal",
        "uri",
        "path",
        "certificate",
        "outcome",
        "pass",
    ] {
        let mut input = f.attestation_input(&id());
        input["command"][field] = json!("not accepted");
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/investigation-attestations",
                Some(&cookie),
                Some(input)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (field, value) in [
        ("assertion", json!("PASS")),
        ("occurred_at", json!({"clock_id":"caller","ticks_ns":"1"})),
    ] {
        let mut input = f.attestation_input(&id());
        input["command"][field] = value;
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/investigation-attestations",
                Some(&cookie),
                Some(input)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (field, value) in [
        ("disposition", json!("RELEASED")),
        ("reason", json!("SUCCEEDED")),
        ("expected_revision", json!(1)),
        ("outcome", json!("NOT_EXECUTED")),
    ] {
        let mut input = f.disposition_input(&id());
        input["command"][field] = value;
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/recovery-dispositions",
                Some(&cookie),
                Some(input)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for query in [
        format!("operation={operation}&limit=0"),
        format!("operation={operation}&limit=51"),
        format!("operation={operation}&limit=1&after=bad"),
        format!("operation={operation}&limit=1&uri=other"),
    ] {
        assert_eq!(
            f.send(
                "GET",
                &format!("/api/v1/investigation-attestations?{query}"),
                Some(&cookie),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let response = f
        .client
        .post(format!("{}/api/v1/recovery-dispositions", f.origin))
        .header("Cookie", &cookie)
        .json(&f.disposition_input(&id()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(f.port.calls.lock().unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn cached_records_are_read_without_worker_and_preserve_false_authority_and_current_flags() {
    let f = fixture().await;
    *f.port.mode.lock().unwrap() = Mode::Cached;
    let cookie = f.login("lead").await;
    let key = id();
    let input = f.attestation_input(&key);
    let (status, value) = f
        .send(
            "POST",
            "/api/v1/investigation-attestations",
            Some(&cookie),
            Some(input.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["current"], false);
    assert_eq!(value["outcome_changed"], false);
    assert_eq!(value["operation_authorized"], false);
    assert_eq!(value["resource_release_authorized"], false);
    assert_eq!(
        value["reference"],
        json!(f.port.records.attestation.reference().unwrap())
    );
    let next_cookie = f.login("lead").await;
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/investigation-attestations",
            Some(&next_cookie),
            Some(input)
        )
        .await
        .1,
        value
    );
    let (status, receipt) = f
        .send(
            "POST",
            "/api/v1/recovery-dispositions",
            Some(&next_cookie),
            Some(f.disposition_input(&id())),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        receipt["receipt"]["after"]["operation"]["outcome"],
        "UNRESOLVED"
    );
    assert_eq!(
        receipt["receipt"]["after"]["operation"]["disposition"],
        "QUARANTINED"
    );
    assert_eq!(receipt["resource_release_authorized"], false);
    let operation = f.port.records.context.work.operation.id();
    let (_, context) = f
        .send(
            "GET",
            &format!("/api/v1/investigation-context?operation={operation}"),
            Some(&next_cookie),
            None,
        )
        .await;
    assert_eq!(
        context["context_digest"],
        json!(f.port.records.context.digest().unwrap())
    );
    let (_, page) = f
        .send(
            "GET",
            &format!("/api/v1/investigation-attestations?operation={operation}&limit=25"),
            Some(&next_cookie),
            None,
        )
        .await;
    assert_eq!(
        page["items"][0]["attestation"]["id"],
        json!(f.port.records.attestation.id)
    );
    assert!(page["next"].is_null());
    let (_, page) = f
        .send(
            "GET",
            &format!("/api/v1/recovery-dispositions?operation={operation}&limit=25"),
            Some(&next_cookie),
            None,
        )
        .await;
    assert_eq!(
        page["items"][0]["receipt"]["id"],
        json!(f.port.records.receipt.id)
    );
    assert!(page["next"].is_null());
    {
        let calls = f.port.calls.lock().unwrap();
        assert_eq!(calls[0]["method"], "attest-preflight");
        assert_eq!(calls[1]["method"], "attest-get");
        assert_eq!(calls[0]["key"], calls[2]["key"]);
        assert_eq!(calls[0]["input"], calls[2]["input"]);
        assert_ne!(calls[0]["session"], calls[2]["session"]);
    }
    let mut recovered = f.attestation_input(&key);
    recovered["command"]["expected_operation_revision"] = json!("999");
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/investigation-attestations",
            Some(&next_cookie),
            Some(recovered.clone())
        )
        .await
        .1,
        value
    );
    recovered["command"]["note"] = json!("A different semantic request");
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/investigation-attestations",
            Some(&next_cookie),
            Some(recovered)
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    f.finish().await;
}

#[tokio::test]
async fn service_roles_and_revoked_human_roles_cannot_recover_cached_investigation_records() {
    let f = fixture().await;
    *f.port.mode.lock().unwrap() = Mode::Cached;
    let cookie = f.login("lead").await;
    for (revision, roles) in [
        (Counter(1), vec![Role::RecoveryLead, Role::Host]),
        (Counter(2), vec![Role::Observer]),
    ] {
        f.runtime
            .call(Command::PutPrincipal {
                identity: f.root.clone(),
                value: principal("lead", &roles),
                expected: Some(revision),
            })
            .await
            .unwrap();
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/investigation-attestations",
                Some(&cookie),
                Some(f.attestation_input(&id()))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/recovery-dispositions",
                Some(&cookie),
                Some(f.disposition_input(&id()))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(f.port.calls.lock().unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn miswired_response_cannot_claim_operation_or_resource_release_authority() {
    let f = fixture().await;
    *f.port.mode.lock().unwrap() = Mode::InvalidAuthority;
    let cookie = f.login("lead").await;
    let operation = f.port.records.context.work.operation.id();
    for path in [
        format!("/api/v1/investigation-context?operation={operation}"),
        format!(
            "/api/v1/investigation-attestation?operation={operation}&id={}",
            f.port.records.attestation.id
        ),
        format!(
            "/api/v1/recovery-disposition?operation={operation}&id={}",
            f.port.records.receipt.id
        ),
    ] {
        let (status, error) = f.send("GET", &path, Some(&cookie), None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error, json!({"code":"UNAVAILABLE","outcome_unknown":true}));
    }
    f.finish().await;
}
