//! Real mTLS terminal connections and real writer/SQLite. Device readiness is synthetic only.
mod support;
use reqwest::{Client, StatusCode};
use rx_api::{
    auth::{Credentials, LocalAccount, password_hash},
    terminal_https::{HttpsPolicy, TerminalHttps, TlsMaterial},
};
use rx_application::*;
use rx_domain::types::*;
use rx_runtime::{
    application::{Application, Command, Handle, Reply},
    writer::Writer,
};
use rx_storage::SqliteRepository;
use serde_json::{Value, json};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn now() -> TimePoint {
    TimePoint {
        clock_id: "terminal-test-clock".into(),
        ticks_ns: Counter(1000),
    }
}
struct ClockSource;
impl Clock for ClockSource {
    fn now(&self) -> TimePoint {
        now()
    }
}
struct SimulationOnly;
fn proof() -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([91; 32]),
        schema_id: name("rx.test.qualification.v1"),
        size_bytes: Counter(1),
    }
}
impl QualificationAuthority for SimulationOnly {
    fn verify(&self, c: &CellConfiguration, e: &[ArtifactRef], d: &[Digest]) -> bool {
        c.environment == Environment::Simulation
            && e == [proof()]
            && d == [
                c.definition.sha256,
                c.envelope.sha256,
                c.recipe.sha256,
                c.site_config_digest,
            ]
    }
}
type Runtime = Handle<Application<SqliteRepository, ClockSource, SimulationOnly>>;
struct Fixture {
    _directory: tempfile::TempDir,
    runtime: Runtime,
    root: Identity,
    origin: String,
    a: Client,
    b: Client,
    unknown: Client,
    no_certificate: Client,
    untrusted: Client,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    configuration: CellConfiguration,
    terminal: Terminal,
}
fn principal(label: &str, roles: &[Role], cells: &[&str]) -> Principal {
    Principal {
        id: name(label),
        client_namespace: name(&format!("client/{label}")),
        roles: roles.iter().copied().collect(),
        cells: cells.iter().map(|s| name(s)).collect(),
        active: true,
    }
}
const PASSWORD: &str = "test-terminal-password-only";
fn credentials() -> Credentials {
    static HASH: OnceLock<String> = OnceLock::new();
    Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: vec![LocalAccount {
            principal: name("alice"),
            password_hash: HASH
                .get_or_init(|| password_hash(PASSWORD).unwrap())
                .clone(),
        }],
    }
}
async fn fixture(qualify: bool) -> Fixture {
    use rcgen::*;
    use sha2::Digest as _;
    let ca = || {
        let mut p = CertificateParams::new(vec![]).unwrap();
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
    };
    let root_ca = ca();
    let foreign_ca = ca();
    let leaf = |issuer: &CertifiedIssuer<'_, KeyPair>, server: bool| {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec!["127.0.0.1".into(), "localhost".into()]).unwrap();
        p.extended_key_usages = vec![if server {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (p.signed_by(&key, issuer).unwrap(), key)
    };
    let (server, key) = leaf(&root_ca, true);
    let (cert_a, key_a) = leaf(&root_ca, false);
    let (cert_b, key_b) = leaf(&root_ca, false);
    let (unknown, unknown_key) = leaf(&root_ca, false);
    let (untrusted, untrusted_key) = leaf(&foreign_ca, false);
    let fingerprint =
        |c: &Certificate| Digest::from_bytes(sha2::Sha256::digest(c.der().as_ref()).into());
    let terminal = Terminal {
        id: name("panel/a"),
        certificate_digest: fingerprint(&cert_a),
        cells: [name("cell/a")].into_iter().collect(),
        active: true,
    };
    let other = Terminal {
        id: name("panel/b"),
        certificate_digest: fingerprint(&cert_b),
        cells: [name("cell/b")].into_iter().collect(),
        active: true,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("platform.db");
    let root_session = id();
    let sid = root_session.clone();
    let configuration = support::configuration("cell/a");
    let config = configuration.clone();
    let t = terminal.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(path)?,
            ClockSource,
            SimulationOnly,
            id(),
            principal(
                "root",
                &[
                    Role::AccountAdmin,
                    Role::Engineer,
                    Role::Verifier,
                    Role::Observer,
                ],
                &["cell/a", "cell/b"],
            ),
        )?;
        engine.authenticated_session(
            &name("root"),
            sid.clone(),
            TimePoint {
                clock_id: now().clock_id,
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let root = Identity {
            principal: name("root"),
            session: sid,
            terminal: None,
        };
        engine.put_principal(
            &root,
            principal(
                "alice",
                &[Role::Operator, Role::Observer],
                &["cell/a", "cell/b"],
            ),
            None,
        )?;
        engine.put_terminal(&root, t, None)?;
        engine.put_terminal(&root, other, None)?;
        engine.install_cell(&root, config.clone())?;
        engine.install_cell(&root, support::configuration("cell/b"))?;
        if qualify {
            engine.qualify(
                &root,
                &config.id,
                Counter(1),
                vec![proof()],
                vec![
                    config.definition.sha256,
                    config.envelope.sha256,
                    config.recipe.sha256,
                    config.site_config_digest,
                ],
            )?;
            engine.put_principal(
                &root,
                principal("executor", &[Role::Executor], &["cell/a"]),
                None,
            )?;
            engine.authenticated_session(
                &name("executor"),
                id(),
                TimePoint {
                    clock_id: now().clock_id,
                    ticks_ns: Counter(u64::MAX),
                },
            )?;
            engine.put_principal(
                &root,
                principal("host/sim", &[Role::Host], &["cell/a"]),
                None,
            )?;
            let h = engine.authenticated_session(
                &name("host/sim"),
                id(),
                TimePoint {
                    clock_id: now().clock_id,
                    ticks_ns: Counter(u64::MAX),
                },
            )?;
            let host = Identity {
                principal: name("host/sim"),
                session: h.id.clone(),
                terminal: None,
            };
            let source = id();
            let scopes = config
                .scopes
                .iter()
                .map(|s| (s.clone(), Counter(1)))
                .collect();
            engine.register_host(
                &host,
                HostRegistration {
                    id: host.principal.clone(),
                    session: h.id,
                    boot_id: id(),
                    delivery_journal: id(),
                    cell: config.id.clone(),
                    epoch: Counter(1),
                    scopes,
                    source_sessions: [(name("ready"), source.clone())].into_iter().collect(),
                    grant: Grant {
                        id: id(),
                        fence: Counter(1),
                        resources: config.steps[0].intent.resource_set.clone(),
                        owner: name(engine.installation.id.as_str()),
                        valid_until: TimePoint {
                            clock_id: now().clock_id,
                            ticks_ns: Counter(10_000_000_000),
                        },
                        ttl_ms: Counter(1000),
                    },
                },
            )?;
            engine.report_fact(
                &host,
                FactRecord {
                    cell: config.id.clone(),
                    id: name("ready"),
                    source_host: host.principal.clone(),
                    source_generation: source,
                    schema: name("boolean/v1"),
                    unit: name("unitless"),
                    acquired_at: now(),
                    maximum_age_ns: config.fact_specs[0].maximum_age_ns,
                    acquisition_uncertainty_ns: Counter(0),
                    quality_good: true,
                    origin_age_bounded: true,
                    disputed: false,
                    value: TypedValue::Boolean(true),
                    evidence_id: id(),
                },
            )?;
        }
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let runtime = Handle::new(writer);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let https = TerminalHttps::new(
        Arc::new(runtime.clone()),
        credentials(),
        HttpsPolicy::new(&origin).unwrap(),
        TlsMaterial {
            server_certificate_pem: server.pem().into_bytes(),
            server_key_pem: key.serialize_pem().into_bytes(),
            terminal_ca_pem: root_ca.pem().into_bytes(),
        },
    )
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(https.serve(listener, async {
        let _ = stopped.await;
    }));
    let client = |identity: Option<(&Certificate, &KeyPair)>| {
        let mut builder = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .tls_built_in_root_certs(false)
            .add_root_certificate(
                reqwest::Certificate::from_pem(root_ca.pem().as_bytes()).unwrap(),
            );
        if let Some((certificate, key)) = identity {
            builder = builder.identity(
                reqwest::Identity::from_pem(
                    format!("{}{}", certificate.pem(), key.serialize_pem()).as_bytes(),
                )
                .unwrap(),
            );
        }
        builder.build().unwrap()
    };
    Fixture {
        _directory: directory,
        runtime,
        root: Identity {
            principal: name("root"),
            session: root_session,
            terminal: None,
        },
        origin,
        a: client(Some((&cert_a, &key_a))),
        b: client(Some((&cert_b, &key_b))),
        unknown: client(Some((&unknown, &unknown_key))),
        no_certificate: client(None),
        untrusted: client(Some((&untrusted, &untrusted_key))),
        stop,
        task,
        configuration,
        terminal,
    }
}
async fn post(
    f: &Fixture,
    client: &Client,
    path: &str,
    cookie: Option<&str>,
    body: Value,
) -> reqwest::Response {
    let mut request = client
        .post(format!("{}{path}", f.origin))
        .header("origin", &f.origin)
        .header("x-rx-client", "browser-v1")
        .json(&body);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    request.send().await.unwrap()
}
async fn login(f: &Fixture, client: &Client) -> (String, Value) {
    let response = post(
        f,
        client,
        "/api/v1/session",
        None,
        json!({"principal":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let header = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        header.contains("HttpOnly")
            && header.contains("SameSite=Strict")
            && header.contains("Secure")
    );
    let cookie = header.split(';').next().unwrap().to_owned();
    (cookie, response.json().await.unwrap())
}
async fn finish(f: Fixture) {
    f.stop.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(8), f.task)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn tls_certificate_and_registry_are_both_required_and_cookie_is_bound_to_terminal() {
    let f = fixture(false).await;
    for c in [&f.no_certificate, &f.untrusted] {
        assert!(
            c.get(format!("{}/api/v1/health", f.origin))
                .send()
                .await
                .is_err()
        );
    }
    let rejected = post(
        &f,
        &f.unknown,
        "/api/v1/session",
        None,
        json!({"principal":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert!(rejected.headers().get("set-cookie").is_none());
    let spoof = f
        .unknown
        .post(format!("{}/api/v1/session", f.origin))
        .header("origin", &f.origin)
        .header("x-rx-client", "browser-v1")
        .header(
            "x-forwarded-client-cert",
            f.terminal.certificate_digest.to_string(),
        )
        .header("x-rx-terminal", "panel/a")
        .json(&json!({"principal":"alice","password":PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(spoof.status(), StatusCode::FORBIDDEN);
    let (cookie, profile) = login(&f, &f.a).await;
    assert_eq!(profile["terminal"], "panel/a");
    assert_eq!(profile["cells"], json!(["cell/a"]));
    let replay =
        f.b.get(format!("{}/api/v1/overview", f.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    let (_, b_profile) = login(&f, &f.b).await;
    assert_eq!(b_profile["cells"], json!(["cell/b"]));
    assert_eq!(
        f.a.get(format!("{}/api/cell/v1/cells/cell%2Fb/inspect", f.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let forged = post(
        &f,
        &f.a,
        "/api/v1/session",
        None,
        json!({"principal":"alice","password":PASSWORD,"terminal":"panel/b"}),
    )
    .await;
    assert_eq!(forged.status(), StatusCode::BAD_REQUEST);
    let new_login = post(
        &f,
        &f.a,
        "/api/v1/session",
        Some(&format!("rx_session={}", "0".repeat(64))),
        json!({"principal":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(new_login.status(), StatusCode::OK);
    let logout = post(&f, &f.a, "/api/v1/session/end", Some(&cookie), json!({})).await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        logout.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    assert_eq!(
        f.a.get(format!("{}/api/v1/overview", f.origin))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    finish(f).await;
}

#[tokio::test]
async fn terminal_bound_operator_can_prepare_start_but_revocation_blocks_cached_requests() {
    let f = fixture(true).await;
    let (cookie, _) = login(&f, &f.a).await;
    let response =
        f.a.get(format!("{}/api/cell/v1/cells/cell%2Fa/inspect", f.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
    let cell: Value = response.json().await.unwrap();
    assert_eq!(cell["commissioning"], "COMMISSIONED");
    let create = json!({"request_key":id(),"command":{"cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,"site_config_digest":f.configuration.site_config_digest,"expected_cell":cell["revision"]}});
    let response = post(&f, &f.a, "/api/v1/runs", Some(&cookie), create).await;
    assert_eq!(response.status(), StatusCode::OK);
    let run: Value = response.json().await.unwrap();
    let start = json!({"request_key":id(),"command":{"run":run["id"],"envelope_digest":f.configuration.envelope.sha256,"purpose":"PRODUCTION","budget_unit":"PART_ATTEMPT","budget_limit":"1","expected_cell":cell["revision"],"expected_run":"1"}});
    let response = post(&f, &f.a, "/api/v1/runs/start", Some(&cookie), start.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let attempt: Value = response.json().await.unwrap();
    assert_eq!(attempt["status"], "ARMING");
    assert_eq!(attempt["actor"], "alice");
    assert_eq!(attempt["terminal"][0], "panel/a");
    let again: Value = post(&f, &f.a, "/api/v1/runs/start", Some(&cookie), start.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(again["id"], attempt["id"]);
    let mut terminal = f.terminal.clone();
    terminal.active = false;
    f.runtime
        .call(Command::PutTerminal {
            identity: f.root.clone(),
            terminal,
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    let rejected = post(&f, &f.a, "/api/v1/runs/start", Some(&cookie), start).await;
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let Reply::RunCheckpoint(snapshot) = f
        .runtime
        .call(Command::RunCheckpoint {
            identity: f.root.clone(),
            run: Id::new(run["id"].as_str().unwrap()).unwrap(),
        })
        .await
        .unwrap()
    else {
        panic!("run snapshot")
    };
    assert_ne!(snapshot.run.state, RunState::Executing);
    finish(f).await;
}

#[tokio::test]
async fn authenticated_terminal_cannot_bypass_qualification_or_current_user_role() {
    let f = fixture(false).await;
    let (cookie, _) = login(&f, &f.a).await;
    let create = json!({"request_key":id(),"command":{"cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,"site_config_digest":f.configuration.site_config_digest,"expected_cell":"1"}});
    let response = post(&f, &f.a, "/api/v1/runs", Some(&cookie), create.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let run: Value = response.json().await.unwrap();
    let response=post(&f,&f.a,"/api/v1/runs/start",Some(&cookie),json!({"request_key":id(),"command":{"run":run["id"],"envelope_digest":f.configuration.envelope.sha256,"purpose":"PRODUCTION","budget_unit":"PART_ATTEMPT","budget_limit":"1","expected_cell":"1","expected_run":"1"}})).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let rejected: Value = response.json().await.unwrap();
    assert_eq!(rejected["code"], "NOT_COMMISSIONED");
    f.runtime
        .call(Command::PutPrincipal {
            identity: f.root.clone(),
            value: principal("alice", &[Role::Observer], &["cell/a", "cell/b"]),
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        post(&f, &f.a, "/api/v1/runs", Some(&cookie), create)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let wrong_origin =
        f.a.post(format!("{}/api/v1/session", f.origin))
            .header("origin", "https://elsewhere.invalid")
            .header("x-rx-client", "browser-v1")
            .json(&json!({"principal":"alice","password":PASSWORD}))
            .send()
            .await
            .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    // An unfinished TLS handshake cannot hold server shutdown indefinitely.
    let _unfinished = tokio::net::TcpStream::connect(f.origin.strip_prefix("https://").unwrap())
        .await
        .unwrap();
    finish(f).await;
}
