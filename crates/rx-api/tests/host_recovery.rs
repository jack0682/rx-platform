//! Real terminal TLS and writer authorization; the mock below tests routing only, never recovery.
mod support;
use reqwest::{Client, StatusCode};
use rx_api::{
    auth::{Credentials, LocalAccount, password_hash},
    terminal_https::{HttpsPolicy, TerminalHttps, TlsMaterial},
};
use rx_application::{host_recovery as recovery, *};
use rx_domain::{fault::Rejection, types::*};
use rx_ports::StoreError;
use rx_runtime::{
    application::{Application, Command, Handle, Reply},
    host_recovery::{QueryResult, Service, ServiceFuture, WorkerError},
    writer::{Writer, WriterError},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn principal(label: &str, roles: &[Role]) -> Principal {
    Principal {
        id: name(label),
        client_namespace: name(&format!("test/{label}")),
        roles: roles.iter().copied().collect(),
        cells: [name("cell/a")].into(),
        active: true,
    }
}
struct ClockSource;
impl Clock for ClockSource {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "recovery-route-test".into(),
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
type Runtime = Handle<Application<rx_storage::SqliteRepository, ClockSource, Deny>>;

#[derive(Clone, Copy)]
enum Failure {
    Busy,
    Transport,
    InvalidRead,
    Stale,
    WriterUnavailable,
}
#[derive(Clone, Copy)]
enum ProposalReply {
    Diagnostic,
    WrongHost,
    WrongOrigin,
    WrongDigest,
    WrongCells,
    WrongPrincipal,
}
struct RoutingMock {
    calls: Mutex<Vec<Value>>,
    failure: Mutex<Failure>,
    context: Mutex<Option<recovery::Context>>,
    query: Mutex<Option<QueryResult>>,
    proposal: Mutex<Option<ProposalReply>>,
    cached_proposal: Mutex<Option<recovery::View>>,
}
impl RoutingMock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(vec![]),
            failure: Mutex::new(Failure::Busy),
            context: Mutex::new(None),
            query: Mutex::new(None),
            proposal: Mutex::new(None),
            cached_proposal: Mutex::new(None),
        })
    }
    fn record(&self, identity: Identity, method: &str, key: Option<Id>, input: Value) {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"method":method,"principal":identity.principal,
            "session":identity.session,"terminal":identity.terminal,"key":key,"input":input}));
    }
    fn answer<T: Send + 'static>(
        &self,
        identity: Identity,
        method: &str,
        key: Option<Id>,
        input: Value,
    ) -> ServiceFuture<'_, T> {
        self.record(identity, method, key, input);
        let error = match *self.failure.lock().unwrap() {
            Failure::Busy => WorkerError::Busy,
            Failure::Transport => WorkerError::Unavailable,
            Failure::InvalidRead => WorkerError::InvalidRead,
            Failure::Stale => WorkerError::Writer(WriterError::Rejected(StoreError::Rejected(
                Rejection::StaleRevision,
            ))),
            Failure::WriterUnavailable => WorkerError::Writer(WriterError::Unavailable),
        };
        Box::pin(async move { Err(error) })
    }
}
impl Service for RoutingMock {
    fn list(
        &self,
        identity: Identity,
        host: Name,
        after: Option<Id>,
        limit: usize,
    ) -> ServiceFuture<'_, recovery::RecoveryPage> {
        self.answer(
            identity,
            "list",
            None,
            json!({"host":host,"after":after,"limit":limit}),
        )
    }
    fn context(
        &self,
        identity: Identity,
        host: Name,
        origin: Name,
    ) -> ServiceFuture<'_, recovery::Context> {
        if let Some(value) = self.context.lock().unwrap().clone() {
            self.record(
                identity,
                "context",
                None,
                json!({"host":host,"origin":origin}),
            );
            return Box::pin(async move { Ok(value) });
        }
        self.answer(
            identity,
            "context",
            None,
            json!({"host":host,"origin":origin}),
        )
    }
    fn propose(
        &self,
        identity: Identity,
        key: Id,
        input: recovery::Prepare,
    ) -> ServiceFuture<'_, recovery::View> {
        if let Some(value) = self.cached_proposal.lock().unwrap().clone() {
            self.record(
                identity,
                "propose",
                Some(key),
                serde_json::to_value(input).unwrap(),
            );
            return Box::pin(async move { Ok(value) });
        }
        if let Some(reply) = *self.proposal.lock().unwrap() {
            let value = proposal_view(
                &identity,
                &input,
                reply,
                self.context.lock().unwrap().clone(),
            );
            self.record(
                identity,
                "propose",
                Some(key),
                serde_json::to_value(input).unwrap(),
            );
            return Box::pin(async move { Ok(value) });
        }
        self.answer(
            identity,
            "propose",
            Some(key),
            serde_json::to_value(input).unwrap(),
        )
    }
    fn approve(
        &self,
        identity: Identity,
        key: Id,
        input: recovery::Approve,
    ) -> ServiceFuture<'_, recovery::View> {
        self.answer(
            identity,
            "approve",
            Some(key),
            serde_json::to_value(input).unwrap(),
        )
    }
    fn get(&self, identity: Identity, id: Id) -> ServiceFuture<'_, recovery::View> {
        self.answer(identity, "get", None, json!({"id":id}))
    }
    fn progress(&self, identity: Identity, id: Id) -> ServiceFuture<'_, recovery::View> {
        self.answer(identity, "progress", None, json!({"id":id}))
    }
    fn query(&self, identity: Identity, id: Id, operation: Id) -> ServiceFuture<'_, QueryResult> {
        if let Some(value) = self.query.lock().unwrap().clone() {
            self.record(
                identity,
                "query",
                None,
                json!({"id":id,"operation":operation}),
            );
            return Box::pin(async move { Ok(value) });
        }
        self.answer(
            identity,
            "query",
            None,
            json!({"id":id,"operation":operation}),
        )
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: Runtime,
    root: Identity,
    client: Client,
    other_client: Client,
    no_certificate: Client,
    origin: String,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
async fn fixture(worker: Option<Arc<dyn Service>>) -> Fixture {
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
    let (other_certificate, other_key) = leaf(false);
    let fingerprint = Digest::from_bytes(sha2::Sha256::digest(certificate.der().as_ref()).into());
    let other_fingerprint =
        Digest::from_bytes(sha2::Sha256::digest(other_certificate.der().as_ref()).into());
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("platform.db");
    let root_session = id();
    let session = root_session.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            rx_storage::SqliteRepository::open(database)?,
            ClockSource,
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
                clock_id: "recovery-route-test".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let root = Identity {
            principal: name("root"),
            session,
            terminal: None,
        };
        for (label, role) in [
            ("release", Role::ReleaseManager),
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
        engine.put_terminal(
            &root,
            Terminal {
                id: name("panel/b"),
                certificate_digest: other_fingerprint,
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    static HASH: OnceLock<String> = OnceLock::new();
    let hash = HASH.get_or_init(|| password_hash("routing-test-password-only").unwrap());
    let credentials = Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: ["release", "operator", "engineer"]
            .into_iter()
            .map(|label| LocalAccount {
                principal: name(label),
                password_hash: hash.clone(),
            })
            .collect(),
    };
    let https = TerminalHttps::new_with_host_recovery(
        Arc::new(runtime.clone()),
        credentials,
        HttpsPolicy::new(&origin).unwrap(),
        TlsMaterial {
            server_certificate_pem: server.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            terminal_ca_pem: ca.pem().into_bytes(),
        },
        None,
        None,
        worker,
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
        client,
        other_client: builder()
            .identity(
                reqwest::Identity::from_pem(
                    format!("{}{}", other_certificate.pem(), other_key.serialize_pem()).as_bytes(),
                )
                .unwrap(),
            )
            .build()
            .unwrap(),
        no_certificate: builder().build().unwrap(),
        origin,
        stop,
        task,
    }
}
impl Fixture {
    async fn login(&self, who: &str) -> String {
        self.login_with(&self.client, who).await
    }
    async fn login_with(&self, client: &Client, who: &str) -> String {
        let response = client
            .post(format!("{}/api/v1/session", self.origin))
            .header("Origin", &self.origin)
            .header("X-RX-Client", "browser-v1")
            .json(&json!({"principal":who,"password":"routing-test-password-only"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("Secure"));
        cookie.split(';').next().unwrap().to_owned()
    }
    async fn send(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.send_with(&self.client, method, path, cookie, body)
            .await
    }
    async fn send_with(
        &self,
        client: &Client,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = client.request(method.parse().unwrap(), format!("{}{path}", self.origin));
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie);
        }
        if let Some(body) = body {
            request = request
                .header("Origin", &self.origin)
                .header("X-RX-Client", "browser-v1")
                .json(&body);
        }
        let response = request.send().await.unwrap();
        (response.status(), response.json().await.unwrap())
    }
    async fn finish(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap().unwrap();
        self.runtime.close();
        self.runtime.closed().await;
    }
}
fn prepare() -> Value {
    json!({"request_key":id(),"command":{"host":"host/sim","origin":"cell/a",
        "expected_context":Digest::from_bytes([7;32]),"expected_cells":{"cell/a":"2"}}})
}

#[tokio::test]
async fn absence_is_reported_only_after_current_terminal_and_release_role_authentication() {
    let f = fixture(None).await;
    let binding = id();
    for (method, path, body) in [
        (
            "GET",
            "/api/v1/host-recoveries?host=host%2Fsim&limit=25".into(),
            None,
        ),
        (
            "GET",
            "/api/v1/host-recovery-context?host=host%2Fsim&origin=cell%2Fa".into(),
            None,
        ),
        ("GET", format!("/api/v1/host-recovery?id={binding}"), None),
        ("POST", "/api/v1/host-recoveries".into(), Some(prepare())),
        (
            "POST",
            "/api/v1/host-recovery/approve".into(),
            Some(
                json!({"request_key":id(),"command":{"id":binding,"expected_revision":"1","proposal_digest":Digest::from_bytes([8;32]),"expected_cells":{"cell/a":"2"}}}),
            ),
        ),
        (
            "POST",
            "/api/v1/host-recovery/progress".into(),
            Some(json!({"id":binding})),
        ),
        (
            "POST",
            "/api/v1/host-recovery/query".into(),
            Some(json!({"id":binding,"operation":id()})),
        ),
    ] {
        assert_eq!(
            f.send(method, &path, None, body).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert!(
        f.no_certificate
            .get(format!("{}/api/v1/host-recovery?id={binding}", f.origin))
            .send()
            .await
            .is_err()
    );
    for who in ["operator", "engineer"] {
        let cookie = f.login(who).await;
        assert_eq!(
            f.send(
                "GET",
                &format!("/api/v1/host-recovery?id={binding}"),
                Some(&cookie),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let cookie = f.login("release").await;
    let (status, error) = f
        .send(
            "GET",
            &format!("/api/v1/host-recovery?id={binding}"),
            Some(&cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error,
        json!({"code":"HOST_RECOVERY_NOT_CONFIGURED","outcome_unknown":false})
    );
    f.finish().await;
}

#[tokio::test]
async fn recovery_discovery_requires_current_role_bounds_and_forwards_the_exact_cursor() {
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let after = id();
    let path = format!("/api/v1/host-recoveries?host=host%2Fsim&after={after}&limit=50");
    assert_eq!(
        f.send("GET", &path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let operator = f.login("operator").await;
    assert_eq!(
        f.send("GET", &path, Some(&operator), None).await.0,
        StatusCode::FORBIDDEN
    );
    let release = f.login("release").await;
    for query in [
        "host=host%2Fsim&limit=0",
        "host=host%2Fsim&limit=51",
        "host=host%2Fsim&limit=-1",
        "host=host%2Fsim",
        "host=host%2Fsim&limit=1&after=not-a-uuid",
        "host=host%2Fsim&limit=1&path=/other",
    ] {
        assert_eq!(
            f.send(
                "GET",
                &format!("/api/v1/host-recoveries?{query}"),
                Some(&release),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(mock.calls.lock().unwrap().is_empty());
    assert_eq!(
        f.send("GET", &path, Some(&release), None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    {
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["method"], "list");
        assert_eq!(
            calls[0]["input"],
            json!({"host":"host/sim","after":after,"limit":50})
        );
        assert_eq!(calls[0]["principal"], "release");
        assert_eq!(calls[0]["terminal"][0], "panel/a");
    }
    f.finish().await;
}

#[tokio::test]
async fn recovery_requests_reject_extra_authority_material_and_reuse_csrf_policy() {
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let cookie = f.login("release").await;
    for field in [
        "snapshot",
        "uri",
        "certificate",
        "path",
        "method",
        "identity",
        "roles",
    ] {
        let mut value = prepare();
        value["command"][field] = json!("untrusted");
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/host-recoveries",
                Some(&cookie),
                Some(value)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for query in [
        "host=host%2Fsim&origin=cell%2Fa&uri=https%3A%2F%2Fother",
        "host=host%2Fsim&host=other&origin=cell%2Fa",
    ] {
        assert_eq!(
            f.send(
                "GET",
                &format!("/api/v1/host-recovery-context?{query}"),
                Some(&cookie),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (path, body) in [
        (
            "/api/v1/host-recovery/progress",
            json!({"id":id(),"request_key":id()}),
        ),
        (
            "/api/v1/host-recovery/query",
            json!({"id":id(),"operation":id(),"method":"Authorize"}),
        ),
        (
            "/api/v1/host-recovery/query",
            json!({"id":"not-a-uuid","operation":id()}),
        ),
        (
            "/api/v1/host-recovery/approve",
            json!({"request_key":id(),"command":{"id":id(),"expected_revision":1,"proposal_digest":Digest::from_bytes([8;32]),"expected_cells":{"cell/a":"2"}}}),
        ),
    ] {
        assert_eq!(
            f.send("POST", path, Some(&cookie), Some(body)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let response = f
        .client
        .post(format!("{}/api/v1/host-recovery/progress", f.origin))
        .header("Cookie", &cookie)
        .json(&json!({"id":id()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(mock.calls.lock().unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn exact_keys_cas_and_existing_operation_ids_reach_only_the_configured_service() {
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let cookie = f.login("release").await;
    let proposal = prepare();
    let binding = id();
    let operation = id();
    let approval = json!({"request_key":id(),"command":{"id":binding,"expected_revision":"9",
        "proposal_digest":Digest::from_bytes([8;32]),"expected_cells":{"cell/a":"2"}}});
    for (method, path, body) in [
        (
            "GET",
            "/api/v1/host-recovery-context?host=host%2Fsim&origin=cell%2Fa".into(),
            None,
        ),
        (
            "POST",
            "/api/v1/host-recoveries".into(),
            Some(proposal.clone()),
        ),
        (
            "POST",
            "/api/v1/host-recoveries".into(),
            Some(proposal.clone()),
        ),
        (
            "POST",
            "/api/v1/host-recovery/approve".into(),
            Some(approval.clone()),
        ),
        ("GET", format!("/api/v1/host-recovery?id={binding}"), None),
        (
            "POST",
            "/api/v1/host-recovery/progress".into(),
            Some(json!({"id":binding})),
        ),
        (
            "POST",
            "/api/v1/host-recovery/query".into(),
            Some(json!({"id":binding,"operation":operation})),
        ),
    ] {
        let (status, error) = f.send(method, &path, Some(&cookie), body).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            error,
            json!({"code":"HOST_RECOVERY_BUSY","outcome_unknown":false})
        );
    }
    let calls = mock.calls.lock().unwrap().clone();
    assert_eq!(
        calls
            .iter()
            .map(|v| v["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "context", "propose", "propose", "approve", "get", "progress", "query"
        ]
    );
    assert_eq!(calls[1], calls[2]);
    assert_eq!(calls[1]["key"], proposal["request_key"]);
    assert_eq!(calls[1]["input"], proposal["command"]);
    assert_eq!(calls[3]["key"], approval["request_key"]);
    assert_eq!(calls[3]["input"], approval["command"]);
    assert_eq!(
        calls[6]["input"],
        json!({"id":binding,"operation":operation})
    );
    assert!(calls.iter().all(|v| v["principal"] == "release"
        && v["terminal"][0] == "panel/a"
        && v["session"] == calls[0]["session"]));
    // A retained cookie is not a cached role grant.
    assert!(matches!(
        f.runtime
            .call(Command::PutPrincipal {
                identity: f.root.clone(),
                value: principal("release", &[Role::Observer]),
                expected: Some(Counter(1))
            })
            .await
            .unwrap(),
        Reply::Revision(_)
    ));
    assert_eq!(
        f.send(
            "GET",
            &format!("/api/v1/host-recovery?id={binding}"),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(mock.calls.lock().unwrap().len(), calls.len());
    f.finish().await;
}

#[tokio::test]
async fn writer_results_keep_their_existing_codes_and_transport_errors_are_sanitized() {
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let cookie = f.login("release").await;
    for (failure, status, code, unknown) in [
        (
            Failure::Stale,
            StatusCode::CONFLICT,
            "STALE_REVISION",
            false,
        ),
        (
            Failure::WriterUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            true,
        ),
        (
            Failure::Transport,
            StatusCode::SERVICE_UNAVAILABLE,
            "HOST_RECOVERY_UNAVAILABLE",
            true,
        ),
        (
            Failure::InvalidRead,
            StatusCode::CONFLICT,
            "HOST_RECOVERY_INVALID_READ",
            false,
        ),
    ] {
        *mock.failure.lock().unwrap() = failure;
        let (actual, error) = f
            .send(
                "POST",
                "/api/v1/host-recovery/progress",
                Some(&cookie),
                Some(json!({"id":id()})),
            )
            .await;
        assert_eq!(actual, status);
        assert_eq!(error, json!({"code":code,"outcome_unknown":unknown}));
    }
    f.finish().await;
}

fn routing_context() -> recovery::Context {
    // DTO output fixture only: no transport, Host registration or verified read.
    let digest = Digest::from_bytes([9; 32]);
    let configuration = support::configuration("cell/a");
    recovery::Context {
        installation: id(),
        store_generation: id(),
        runtime_boot: id(),
        clock_id: "recovery-route-test".into(),
        host: name("host/sim"),
        origin: name("cell/a"),
        transport: recovery::TransportPin {
            uri: "https://test.invalid:7444".into(),
            server_name: "test.invalid".into(),
            server_ca_digest: digest,
            server_leaf_digest: digest,
            platform_client_leaf_digest: digest,
            platform_client_chain_digest: digest,
            release: digest,
            base_manifest: digest,
            cell_manifest: digest,
            host_read_binding: digest,
            host_configuration_binding: digest,
        },
        producer: EvidenceProducer {
            principal: name("host/sim"),
            session: id(),
            peer_boot: id(),
            journal: id(),
            authentication_binding: digest,
            cells: [(name("cell/a"), configuration.definition.sha256)].into(),
        },
        producer_revision: Counter(1),
        anchor: None,
        previous_runtime_boot: id(),
        host_cells: vec![name("cell/a")],
        cells: [(
            name("cell/a"),
            recovery::CellCut {
                revision: Counter(7),
                epoch: Counter(3),
                scopes: [(name("zone/shared"), Counter(3))].into(),
                configuration,
                configuration_digest: digest,
                blocks: vec![],
                runtime_origins: Default::default(),
            },
        )]
        .into(),
        registrations: Default::default(),
        operations: Default::default(),
        blockers: vec![recovery::Blocker::BaselineMissing {
            cell: name("cell/a"),
        }],
    }
}

#[tokio::test]
async fn context_digests_are_computed_by_the_server_and_diagnostic_queries_cannot_claim_authority()
{
    let mock = RoutingMock::new();
    let context = routing_context();
    *mock.context.lock().unwrap() = Some(context.clone());
    let f = fixture(Some(mock.clone())).await;
    let cookie = f.login("release").await;
    let (status, value) = f
        .send(
            "GET",
            "/api/v1/host-recovery-context?host=host%2Fsim&origin=cell%2Fa",
            Some(&cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["context"], serde_json::to_value(&context).unwrap());
    assert_eq!(value["context_digest"], json!(context.digest().unwrap()));
    assert_eq!(value["expected_cells"], json!({"cell/a":"7"}));
    let binding = id();
    let operation = id();
    let query = QueryResult {
        binding: binding.clone(),
        operation: operation.clone(),
        receipt: None,
        lookup: recovery::QueryLookup::Unsupported,
        evidence: vec![],
        evidence_complete: false,
        publication_required: true,
        operation_authorized: false,
    };
    *mock.query.lock().unwrap() = Some(query.clone());
    let input = json!({"id":binding,"operation":operation});
    let (status, value) = f
        .send(
            "POST",
            "/api/v1/host-recovery/query",
            Some(&cookie),
            Some(input.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, serde_json::to_value(&query).unwrap());
    for operation_authorized in [false, true] {
        let mut invalid = query.clone();
        invalid.operation_authorized = operation_authorized;
        invalid.evidence_complete = !operation_authorized;
        *mock.query.lock().unwrap() = Some(invalid);
        assert_eq!(
            f.send(
                "POST",
                "/api/v1/host-recovery/query",
                Some(&cookie),
                Some(input.clone())
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    let mut wrong = query;
    wrong.operation = id();
    *mock.query.lock().unwrap() = Some(wrong);
    assert_eq!(
        f.send(
            "POST",
            "/api/v1/host-recovery/query",
            Some(&cookie),
            Some(input)
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    f.finish().await;
}

fn proposal_view(
    actor: &Identity,
    input: &recovery::Prepare,
    reply: ProposalReply,
    original: Option<recovery::Context>,
) -> recovery::View {
    use rx_domain::host_configuration::{Observation, Snapshot};
    // This is a routing response fixture, not a VerifiedRead or an admitted core binding.
    let mut context = original.unwrap_or_else(routing_context);
    context.blockers.push(recovery::Blocker::BaselineMissing {
        cell: input.origin.clone(),
    });
    context.host = input.host.clone();
    context.origin = input.origin.clone();
    for (cell, revision) in &input.expected_cells {
        context.cells.get_mut(cell).unwrap().revision = *revision;
    }
    let at = ClockSource.now();
    let proposal_read = recovery::ReadEvidence {
        platform_session: id(),
        transport: context.transport.clone(),
        configuration: Observation {
            schema: name("rx.host-process-configuration-observation.v1"),
            snapshot: Snapshot {
                schema: name("rx.host-process-configuration-snapshot.v1"),
                host: input.host.clone(),
                host_boot: id(),
                delivery_journal: id(),
                binding_digest: Digest::from_bytes([9; 32]),
                cells: vec![],
            },
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        },
        configuration_started: at.clone(),
        configuration_finished: at.clone(),
        cells: Default::default(),
    };
    let mut binding = recovery::Binding {
        schema: name(recovery::SCHEMA),
        id: id(),
        revision: Counter(1),
        phase: recovery::Phase::Proposed,
        requested_context_digest: input.expected_context,
        context,
        proposed_by: recovery::StoredActor::from(actor),
        proposed_at: at,
        proposal_read,
        approved_by: None,
        approved_at: None,
        fences: Default::default(),
        last_read: None,
        detail: None,
    };
    match reply {
        ProposalReply::Diagnostic => {}
        ProposalReply::WrongHost => binding.context.host = name("host/other"),
        ProposalReply::WrongOrigin => binding.context.origin = name("cell/other"),
        ProposalReply::WrongDigest => {
            binding.requested_context_digest = Digest::from_bytes([42; 32])
        }
        ProposalReply::WrongCells => {
            binding
                .context
                .cells
                .get_mut(&name("cell/a"))
                .unwrap()
                .revision = Counter(99)
        }
        ProposalReply::WrongPrincipal => {
            binding.proposed_by.principal = name("another-release-manager")
        }
    }
    recovery::View {
        binding,
        current: false,
        operation_authorized: false,
    }
}

#[tokio::test]
async fn proposal_response_matches_the_original_request_and_actor_even_when_diagnostic_blockers_change()
 {
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let cookie = f.login("release").await;
    let mut original = routing_context();
    original.blockers.clear();
    *mock.context.lock().unwrap() = Some(original.clone());
    let expected_context = original.digest().unwrap();
    let input = json!({"request_key":id(),"command":{"host":"host/sim","origin":"cell/a",
        "expected_context":expected_context,"expected_cells":original.expected_cells()}});
    *mock.proposal.lock().unwrap() = Some(ProposalReply::Diagnostic);
    let (status, value) = f
        .send(
            "POST",
            "/api/v1/host-recoveries",
            Some(&cookie),
            Some(input.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let binding: recovery::Binding =
        serde_json::from_value(value["view"]["binding"].clone()).unwrap();
    assert_eq!(binding.requested_context_digest, expected_context);
    assert!(!binding.context.blockers.is_empty());
    assert_ne!(binding.context.digest().unwrap(), expected_context);
    assert_eq!(
        value["proposal_digest"],
        json!(binding.proposal_digest().unwrap())
    );
    for reply in [
        ProposalReply::WrongHost,
        ProposalReply::WrongOrigin,
        ProposalReply::WrongDigest,
        ProposalReply::WrongCells,
        ProposalReply::WrongPrincipal,
    ] {
        *mock.proposal.lock().unwrap() = Some(reply);
        let (status, error) = f
            .send(
                "POST",
                "/api/v1/host-recoveries",
                Some(&cookie),
                Some(input.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error, json!({"code":"UNAVAILABLE","outcome_unknown":true}));
    }
    f.finish().await;
}

#[tokio::test]
async fn cached_proposal_keeps_historical_actor_after_relogin_on_another_registered_terminal() {
    // Only the service's cached reply is mocked; both terminal logins and role revocation use
    // the actual writer. Historical identifiers never become current request credentials.
    let mock = RoutingMock::new();
    let f = fixture(Some(mock.clone())).await;
    let first_cookie = f.login("release").await;
    let mut original = routing_context();
    original.blockers.clear();
    *mock.context.lock().unwrap() = Some(original.clone());
    *mock.proposal.lock().unwrap() = Some(ProposalReply::Diagnostic);
    let input = json!({"request_key":id(),"command":{"host":"host/sim","origin":"cell/a",
        "expected_context":original.digest().unwrap(),"expected_cells":original.expected_cells()}});
    let (status, first) = f
        .send(
            "POST",
            "/api/v1/host-recoveries",
            Some(&first_cookie),
            Some(input.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let binding: recovery::Binding =
        serde_json::from_value(first["view"]["binding"].clone()).unwrap();
    assert_eq!(
        binding.proposed_by.terminal.as_ref().unwrap().0,
        name("panel/a")
    );
    *mock.cached_proposal.lock().unwrap() = Some(recovery::View {
        binding: binding.clone(),
        current: false,
        operation_authorized: false,
    });

    let next_cookie = f.login_with(&f.other_client, "release").await;
    assert_ne!(first_cookie, next_cookie);
    let (status, recovered) = f
        .send_with(
            &f.other_client,
            "POST",
            "/api/v1/host-recoveries",
            Some(&next_cookie),
            Some(input.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered, first);
    {
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["principal"], calls[1]["principal"]);
        assert_eq!(calls[0]["key"], calls[1]["key"]);
        assert_eq!(calls[0]["input"], calls[1]["input"]);
        assert_ne!(calls[0]["session"], calls[1]["session"]);
        assert_eq!(calls[0]["terminal"][0], "panel/a");
        assert_eq!(calls[1]["terminal"][0], "panel/b");
        assert_eq!(
            recovered["view"]["binding"]["proposed_by"]["session"],
            calls[0]["session"]
        );
        assert_eq!(
            recovered["view"]["binding"]["proposed_by"]["terminal"],
            calls[0]["terminal"]
        );
    }
    f.runtime
        .call(Command::PutPrincipal {
            identity: f.root.clone(),
            value: principal("release", &[Role::Observer]),
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        f.send_with(
            &f.other_client,
            "POST",
            "/api/v1/host-recoveries",
            Some(&next_cookie),
            Some(input)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(mock.calls.lock().unwrap().len(), 2);
    f.finish().await;
}
