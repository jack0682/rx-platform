use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use rx_api::{
    LocalPolicy,
    auth::{Credentials, LocalAccount, password_hash},
    router,
};
use rx_application::*;
use rx_domain::types::*;
#[path = "support/operator_start_http.rs"]
mod operator_start_http;
mod support;
use rx_runtime::{
    application::{Application, Command, Handle, Reply},
    writer::Writer,
};
use rx_storage::SqliteRepository;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use support::configuration;
use tower::ServiceExt;

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
#[derive(Clone)]
struct ClockSource(Arc<AtomicU64>);
impl Clock for ClockSource {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
        }
    }
}
struct Deny;
impl QualificationAuthority for Deny {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type AppHandle = Handle<Application<SqliteRepository, ClockSource, Deny>>;
struct Fixture {
    _dir: tempfile::TempDir,
    app: Router,
    handle: AppHandle,
    clock: ClockSource,
    admin: Identity,
    reader: Principal,
    configuration: CellConfiguration,
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
fn credentials() -> Credentials {
    static HASH: OnceLock<String> = OnceLock::new();
    let hash = HASH.get_or_init(|| password_hash("test-password-only").unwrap());
    Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: ["admin", "reader", "host/sim"]
            .into_iter()
            .map(|p| LocalAccount {
                principal: name(p),
                password_hash: hash.clone(),
            })
            .collect(),
    }
}
async fn fixture() -> Fixture {
    fixture_with_runtime_restrictions(false).await
}
async fn fixture_with_runtime_restrictions(restart: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.db");
    let clock = ClockSource(Arc::new(AtomicU64::new(1000)));
    let engine_clock = clock.clone();
    let reader = principal("reader", &[Role::Observer], &["cell/a"]);
    let setup_reader = reader.clone();
    let cell_config = configuration("cell/a");
    let setup_config = cell_config.clone();
    let admin_session = id();
    let session = admin_session.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(path)?,
            engine_clock.clone(),
            Deny,
            id(),
            principal(
                "admin",
                &[
                    Role::AccountAdmin,
                    Role::Engineer,
                    Role::Operator,
                    Role::RecoveryLead,
                    Role::Observer,
                ],
                &["cell/a", "cell/b", "cell/c"],
            ),
        )?;
        let setup_session = if restart { id() } else { session.clone() };
        engine.authenticated_session(
            &name("admin"),
            setup_session.clone(),
            TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let admin = Identity {
            principal: name("admin"),
            session: setup_session,
            terminal: None,
        };
        engine.put_principal(&admin, setup_reader, None)?;
        engine.put_principal(
            &admin,
            principal("host/sim", &[Role::Host], &["cell/a"]),
            None,
        )?;
        engine.install_cell(&admin, setup_config)?;
        engine.install_cell(&admin, configuration("cell/b"))?;
        if restart {
            use rx_application::persistence as p;
            use rx_ports::Repository;
            engine.hold(&admin, id().as_str(), &name("cell/a"))?;
            let installation = engine.installation.id.clone();
            let mut repository = engine.into_repository();
            repository.transact(|tx| {
                let (revision, mut cell): (_, Cell) =
                    p::load(tx, "cell", name("cell/a"), "rx.internal.cell.v1")?;
                cell.blocks.push(Block {
                    id: id(),
                    created_revision: Some(Counter(revision.0 + 1)),
                    case_id: None,
                    reason: BlockReason::RuntimeRestart,
                    latched: true,
                    scopes: cell.configuration.scopes.clone(),
                });
                // A legacy non-latched row must never become a selectable runtime restriction.
                cell.blocks.push(Block {
                    id: id(),
                    created_revision: Some(Counter(revision.0 + 1)),
                    case_id: None,
                    reason: BlockReason::RuntimeRestart,
                    latched: false,
                    scopes: cell.configuration.scopes.clone(),
                });
                p::save(
                    tx,
                    "cell",
                    &cell.configuration.id,
                    Some(revision),
                    "rx.internal.cell.v1",
                    &cell,
                )?;
                Ok(())
            })?;
            engine = Engine::open(
                repository,
                engine_clock,
                Deny,
                installation,
                principal(
                    "admin",
                    &[Role::AccountAdmin],
                    &["cell/a", "cell/b", "cell/c"],
                ),
            )?;
            engine.authenticated_session(
                &name("admin"),
                session,
                TimePoint {
                    clock_id: "test-clock".into(),
                    ticks_ns: Counter(u64::MAX),
                },
            )?;
        }
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let handle = Handle::new(writer);
    let app = router(
        Arc::new(handle.clone()),
        credentials(),
        LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
    )
    .unwrap();
    Fixture {
        _dir: dir,
        app,
        handle,
        clock,
        admin: Identity {
            principal: name("admin"),
            session: admin_session,
            terminal: None,
        },
        reader,
        configuration: cell_config,
    }
}
fn request(method: &str, path: &str, cookie: Option<&str>, body: String) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("origin", "http://127.0.0.1:8080")
        .header("content-type", "application/json")
        .header("x-rx-client", "browser-v1");
    if let Some(cookie) = cookie {
        r = r.header(header::COOKIE, cookie);
    }
    let mut request = r.body(Body::from(body)).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:40000".parse::<SocketAddr>().unwrap(),
    ));
    request
}
async fn send(app: &Router, r: Request<Body>) -> (StatusCode, Value, Option<String>) {
    let response = app.clone().oneshot(r).await.unwrap();
    let status = response.status();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, cookie)
}
async fn login(app: &Router, principal: &str) -> String {
    let (status, profile, cookie) = send(
        app,
        request(
            "POST",
            "/api/v1/session",
            None,
            json!({"principal":principal,"password":"test-password-only"}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert!(profile.get("session").is_none());
    let cookie = cookie.unwrap();
    assert!(cookie.contains("HttpOnly; SameSite=Strict"));
    cookie.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn runtime_restriction_selection_is_scoped_read_only_and_keeps_missing_origins_null() {
    let f = fixture_with_runtime_restrictions(true).await;
    let admin_cookie = login(&f.app, "admin").await;
    let path = "/api/v1/runtime-restrictions?cell=cell%2Fa";
    let (_, before, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/cell?id=cell%2Fa",
            Some(&admin_cookie),
            String::new(),
        ),
    )
    .await;
    let (status, value, _) = send(
        &f.app,
        request("GET", path, Some(&admin_cookie), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: rx_application::runtime_invalidation::RuntimeRestrictions =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(view.cell, name("cell/a"));
    assert_eq!(value["revision"], before["revision"]);
    assert_eq!(value["epoch"], before["value"]["epoch"]);
    assert_eq!(
        view.configuration_digest,
        rx_application::runtime_invalidation::configuration_digest(&f.configuration).unwrap()
    );
    assert_eq!(view.restrictions.len(), 2);
    assert!(
        view.restrictions
            .iter()
            .all(|r| r.block.latched && r.block.reason == BlockReason::RuntimeRestart)
    );
    assert_eq!(
        view.restrictions
            .iter()
            .filter(|r| r.origin.is_some())
            .count(),
        1
    );
    for row in value["restrictions"].as_array().unwrap() {
        assert!(row.get("origin").is_some() && row.get("origin_digest").is_some());
        if row["origin"].is_null() {
            assert!(row["origin_digest"].is_null());
        }
    }
    let present = view
        .restrictions
        .iter()
        .find(|r| r.origin.is_some())
        .unwrap();
    assert_eq!(
        present.origin_digest,
        Some(present.origin.as_ref().unwrap().digest().unwrap())
    );
    assert!(!value.to_string().contains("cell/b"));
    let (_, after, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/cell?id=cell%2Fa",
            Some(&admin_cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(before, after);
    assert_eq!(
        send(&f.app, request("GET", path, None, String::new()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    let reader_cookie = login(&f.app, "reader").await;
    assert_eq!(
        send(
            &f.app,
            request("GET", path, Some(&reader_cookie), String::new())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut reader = f.reader.clone();
    reader.roles = [Role::Verifier].into_iter().collect();
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: reader.clone(),
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        send(
            &f.app,
            request("GET", path, Some(&reader_cookie), String::new())
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, denied, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/runtime-restrictions?cell=cell%2Fb",
            Some(&reader_cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(denied.get("restrictions").is_none());
    reader.roles = [Role::Operator].into_iter().collect();
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: reader,
            expected: Some(Counter(2)),
        })
        .await
        .unwrap();
    assert_eq!(
        send(
            &f.app,
            request("GET", path, Some(&reader_cookie), String::new())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let other = fixture().await;
    assert_eq!(
        send(
            &other.app,
            request("GET", path, Some(&admin_cookie), String::new())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.handle.close();
    f.handle.closed().await;
    other.handle.close();
    other.handle.closed().await;
}

#[tokio::test]
async fn scoped_snapshot_login_logout_and_current_revocation() {
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    let (status, overview, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overview["cells"].as_array().unwrap().len(), 1);
    assert_eq!(overview["cells"][0]["cell"]["value"]["id"], "cell/a");
    assert!(overview["cells"][0]["cell"]["value"]["qualification"].is_null());
    assert!(!overview.to_string().contains("cell/b"));
    let diagnostics = &overview["cells"][0]["diagnostics"];
    assert_eq!(diagnostics["schema"], "rx.cell-diagnostics.v1");
    assert_eq!(diagnostics["hosts"][0]["context"], "UNREGISTERED");
    assert_eq!(diagnostics["conditions"][0]["verdict"], "UNKNOWN");
    assert!(diagnostics["sources"][0]["observation"].is_null());
    assert!(
        diagnostics["sources"][0]["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("MISSING_OBSERVATION"))
    );
    assert!(overview.get("seq").is_none());
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                "/api/v1/cell?id=cell%2Fb",
                Some(&cookie),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut reader = f.reader.clone();
    reader.active = false;
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: reader,
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        send(
            &f.app,
            request("GET", "/api/v1/overview", Some(&cookie), String::new())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let admin_cookie = login(&f.app, "admin").await;
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/session/end",
                Some(&admin_cookie),
                "{}".into()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                "/api/v1/overview",
                Some(&admin_cookie),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.handle.close();
    f.handle.closed().await;
}

#[tokio::test]
async fn strict_ingress_and_credentials_never_create_authority_from_headers() {
    let f = fixture().await;
    for principal in ["missing", "admin", "host/sim"] {
        let password = if principal == "host/sim" {
            "test-password-only"
        } else {
            "wrong-password"
        };
        let (status, _, cookie) = send(
            &f.app,
            request(
                "POST",
                "/api/v1/session",
                None,
                json!({"principal":principal,"password":password}).to_string(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(cookie.is_none());
    }
    for body in [
        r#"{"principal":"admin","principal":"reader","password":"test-password-only"}"#,
        r#"{"principal":"admin","password":"test-password-only","roles":["ACCOUNT_ADMIN"]}"#,
    ] {
        assert_eq!(
            send(
                &f.app,
                request("POST", "/api/v1/session", None, body.into())
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for field in ["origin", "host", "x-rx-client"] {
        let mut req = request("POST", "/api/v1/session", None, "{}".into());
        req.headers_mut().remove(field);
        assert_eq!(send(&f.app, req).await.0, StatusCode::FORBIDDEN);
    }
    let mut req = request("GET", "/api/v1/health", None, String::new());
    req.extensions_mut().insert(ConnectInfo(
        "192.168.0.1:40000".parse::<SocketAddr>().unwrap(),
    ));
    assert_eq!(send(&f.app, req).await.0, StatusCode::FORBIDDEN);
    let cookie = login(&f.app, "admin").await;
    let duplicate = format!("{cookie}; {cookie}");
    assert_eq!(
        send(
            &f.app,
            request("GET", "/api/v1/overview", Some(&duplicate), String::new())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.handle.close();
    f.handle.closed().await;
}

#[tokio::test]
async fn mutations_are_durable_idempotent_and_terminal_bypass_is_rejected() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let payload = json!({"request_key":id(),"command":{
        "cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,
        "site_config_digest":f.configuration.site_config_digest,"expected_cell":"1"}});
    let (status, run, _) = send(
        &f.app,
        request("POST", "/api/v1/runs", Some(&cookie), payload.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let repeated = send(
        &f.app,
        request("POST", "/api/v1/runs", Some(&cookie), payload.to_string()),
    )
    .await;
    assert_eq!(repeated.1, run);
    let mut changed = payload.clone();
    changed["command"]["recipe_digest"] = json!(Digest::from_bytes([99; 32]));
    assert_eq!(
        send(
            &f.app,
            request("POST", "/api/v1/runs", Some(&cookie), changed.to_string())
        )
        .await
        .1["code"],
        "KEY_CONFLICT"
    );
    let mut req=request("POST","/api/v1/runs/start",Some(&cookie),json!({"request_key":id(),"command":{
        "run":run["id"],"envelope_digest":f.configuration.envelope.sha256,"purpose":"PRODUCTION","budget_unit":"PART_ATTEMPT",
        "budget_limit":"2","expected_cell":"1","expected_run":"1"}}).to_string());
    req.headers_mut()
        .insert("x-rx-terminal", "panel/main".parse().unwrap());
    req.headers_mut()
        .insert("x-rx-roles", "OPERATOR".parse().unwrap());
    assert_eq!(send(&f.app, req).await.0, StatusCode::FORBIDDEN);
    let installed = json!({"request_key":id(),"command":configuration("cell/c")});
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cells",
                Some(&cookie),
                installed.to_string()
            )
        )
        .await
        .1["revision"],
        "1"
    );
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cells",
                Some(&cookie),
                installed.to_string()
            )
        )
        .await
        .1["revision"],
        "1"
    );
    let hold = json!({"request_key":id(),"command":{"cell":"cell/a"}});
    let first = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cells/hold",
            Some(&cookie),
            hold.to_string(),
        ),
    )
    .await;
    let repeat = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cells/hold",
            Some(&cookie),
            hold.to_string(),
        ),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1, repeat.1);
    assert_eq!(first.1["epoch"], "2");
    f.handle.close();
    f.handle.closed().await;
}

#[tokio::test]
async fn expired_session_and_stopped_writer_never_return_cached_success() {
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    f.clock.0.store(3_600_000_001_001, Ordering::SeqCst);
    assert_eq!(
        send(
            &f.app,
            request("GET", "/api/v1/overview", Some(&cookie), String::new())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.handle.close();
    f.handle.closed().await;
    let reply = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(reply.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.1["outcome_unknown"], true);
}

#[tokio::test]
async fn actual_tcp_listener_uses_verified_socket_origin_and_real_writer() {
    let f = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let app = router(
        Arc::new(f.handle.clone()),
        credentials(),
        LocalPolicy::new(&origin).unwrap(),
    )
    .unwrap();
    let (stop, done) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = done.await;
        })
        .await
        .unwrap()
    });
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{origin}/api/v1/session"))
        .header("origin", &origin)
        .header("x-rx-client", "browser-v1")
        .json(&json!({"principal":"reader","password":"test-password-only"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let response = client
        .get(format!("{origin}/api/v1/overview"))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["cells"].as_array().unwrap().len(), 1);
    stop.send(()).unwrap();
    server.await.unwrap();
    f.handle.close();
    f.handle.closed().await;
}

#[tokio::test]
async fn checkpoint_wire_and_exact_artifact_are_scoped_to_current_user_access() {
    use sha2::{Digest as _, Sha256};
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let payload = json!({"request_key":id(),"command":{
        "cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,
        "site_config_digest":f.configuration.site_config_digest,"expected_cell":"1"}});
    let (status, run, _) = send(
        &f.app,
        request("POST", "/api/v1/runs", Some(&cookie), payload.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run_id = run["id"].as_str().unwrap();
    let path = format!("/api/v1/run/checkpoint?id={run_id}");
    let reader_cookie = login(&f.app, "reader").await;
    let (status, view, _) = send(
        &f.app,
        request("GET", &path, Some(&reader_cookie), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let wire: rx_protocol::base::RunView =
        rx_protocol::json::from_slice(&serde_json::to_vec(&view).unwrap()).unwrap();
    assert_eq!(wire.run_id, run_id);
    assert_eq!(view["state"], "PREPARED");
    assert_eq!(view["revision"], "1");
    assert_eq!(view["checkpoint"]["revision"], "1");
    assert_eq!(view["checkpoint"]["activations"], json!([]));
    assert!(view.get("executor_session_id").is_none());
    let reference = &view["checkpoint"]["payload"];
    let artifact_path = format!(
        "/api/v1/run/checkpoint/artifact?run={run_id}&sha256={}&schema_id={}&size_bytes={}",
        reference["sha256"].as_str().unwrap(),
        reference["schema_id"].as_str().unwrap(),
        reference["size_bytes"].as_str().unwrap()
    );
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            &artifact_path,
            Some(&reader_cookie),
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let digest = Digest::from_bytes(Sha256::digest(&bytes).into());
    assert_eq!(digest.to_string(), reference["sha256"].as_str().unwrap());
    assert_eq!(
        bytes.len().to_string(),
        reference["size_bytes"].as_str().unwrap()
    );
    let state: rx_application::checkpoint_artifact::ExecutorState =
        rx_domain::canonical::decode_json(&bytes).unwrap();
    assert_eq!(state.run.id.to_string(), run_id);
    assert_eq!(state.revision, Counter(1));
    assert_eq!(
        send(&f.app, request("GET", &artifact_path, None, String::new()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for malformed in [
        format!("{path}&id={run_id}"),
        format!("{path}&extra=true"),
        artifact_path.replace("size_bytes=", "size_bytes=0"),
    ] {
        assert_eq!(
            send(
                &f.app,
                request("GET", &malformed, Some(&reader_cookie), String::new())
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut restricted = f.reader.clone();
    restricted.cells.clear();
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: restricted,
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    for restricted_path in [&path, &artifact_path] {
        assert_eq!(
            send(
                &f.app,
                request("GET", restricted_path, Some(&reader_cookie), String::new())
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    f.handle.close();
    f.handle.closed().await;
}

#[tokio::test]
async fn case_acknowledgment_records_reading_without_changing_containment_or_scope() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let body = json!({"request_key":id(),"command":{"cell":"cell/a","kind":"FAULT_RECOVERY","scopes":[],"procedure":{"sha256":"9595959595959595959595959595959595959595959595959595959595959595","schema_id":"rx.test.procedure.v1","size_bytes":"1"},"lead":"admin","operation_ids":[],"material_ids":[]}});
    let (status, opened, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/open",
            Some(&cookie),
            body.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["case"]["state"], "CONTAINMENT_PENDING");
    let before = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await
    .1;
    let command = json!({"request_key":id(),"command":{"cell":"cell/a","case":opened["case"]["id"],"expected_case":opened["revision"],"occurred_at":"2026-09-11T01:02:03Z"}});
    let (status, acked, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/acknowledge",
            Some(&cookie),
            command.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(acked["snapshot"]["case"]["state"], "CONTAINMENT_PENDING");
    assert_eq!(acked["acknowledgments"].as_array().unwrap().len(), 1);
    let (_, repeated, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/acknowledge",
            Some(&cookie),
            command.to_string(),
        ),
    )
    .await;
    assert_eq!(acked, repeated);
    let after = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await
    .1;
    assert_eq!(
        before["cells"][0]["cell"]["value"]["epoch"],
        after["cells"][0]["cell"]["value"]["epoch"]
    );
    assert_eq!(
        before["cells"][0]["cell"]["value"]["blocks"],
        after["cells"][0]["cell"]["value"]["blocks"]
    );
    let reader = login(&f.app, "reader").await;
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/acknowledge",
            Some(&reader),
            command.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, list, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/cases?cell=cell%2Fa",
            Some(&reader),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["cases"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn stale_external_procedure_report_returns_its_durable_fact_ids() {
    use rx_application::procedure::*;
    use sha2::Digest as _;
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let procedure = ArtifactRef {
        sha256: Digest::from_bytes([95; 32]),
        schema_id: name("rx.test.procedure.v1"),
        size_bytes: Counter(1),
    };
    let body = json!({"request_key":id(),"command":{"cell":"cell/a","kind":"DIAGNOSTIC_ONLY","scopes":[],"procedure":procedure,"lead":"admin","operation_ids":[],"material_ids":[]}});
    let (status, opened, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/open",
            Some(&cookie),
            body.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let case: rx_application::intervention::CaseSnapshot = serde_json::from_value(opened).unwrap();
    let ack = json!({"request_key":id(),"command":{"cell":"cell/a","case":case.case.id,"expected_case":case.revision,"occurred_at":"2026-09-11T01:00:00Z"}});
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cases/acknowledge",
                Some(&cookie),
                ack.to_string()
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    let assertions = Assertions {
        schema: name("rx.procedure-assertions.v1"),
        case_id: case.case.id.clone(),
        case_revision: case.revision,
        procedure_digest: procedure.sha256,
        actor: name("admin"),
        action: Action::WorkStarted,
        occurred_at: "2026-09-11T01:01:00Z".into(),
        scope_ids: f.configuration.scopes.clone(),
        source: name("admin"),
        physical_claims: vec![Claim {
            subject: name("fixture/material"),
            predicate: name("manual-work"),
            value: TypedValue::Boolean(true),
        }],
        source_event: Some(id()),
        step: Some(name("work-start")),
        people: vec![name("admin")],
        evidence_ids: vec![],
        observed_at: None,
        valid_until: None,
        certainty: Some(Certainty::Unknown),
    };
    let bytes = rx_domain::canonical::bytes(&assertions).unwrap();
    let reference = ArtifactRef {
        sha256: Digest::from_bytes(sha2::Sha256::digest(&bytes).into()),
        schema_id: name("rx.procedure-assertions.v1"),
        size_bytes: Counter(bytes.len() as u64),
    };
    let record = rx_application::procedure::Record {
        id: id(),
        case: case.case.id.clone(),
        case_revision: case.revision,
        action: Action::WorkStarted,
        actor: name("admin"),
        scope_ids: assertions.scope_ids.clone(),
        occurred_at: assertions.occurred_at.clone(),
        evidence_ids: vec![],
        assertions: reference,
    };
    let command = Submission {
        cell: f.configuration.id.clone(),
        expected_cell: Some(Counter(1)),
        expected_case: case.revision,
        record: record.clone(),
        assertions: Some(assertions),
    };
    let body = json!({"request_key":id(),"command":command}).to_string();
    let (status, result, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/procedure",
            Some(&cookie),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(result["code"], "STALE_REVISION");
    assert_eq!(result["facts_recorded"], true);
    assert_eq!(result["record_id"], record.id.to_string());
    let repeated = send(
        &f.app,
        request("POST", "/api/v1/cases/procedure", Some(&cookie), body),
    )
    .await;
    assert_eq!(result, repeated.1);
    let detail = send(
        &f.app,
        request(
            "GET",
            &format!("/api/v1/case?cell=cell%2Fa&case={}", case.case.id),
            Some(&cookie),
            String::new(),
        ),
    )
    .await
    .1;
    assert_eq!(detail["procedure_records"].as_array().unwrap().len(), 1);
    assert_eq!(detail["snapshot"]["acknowledgment_count"], "1");
    assert_eq!(detail["snapshot"]["procedure_record_count"], "1");
    assert_eq!(detail["snapshot"]["case"]["state"], "ESCALATED");
}

#[tokio::test]
async fn close_http_requires_current_lead_cohort_evidence_and_real_clearance() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let body = json!({"request_key":id(),"command":{"cell":"cell/a","kind":"FAULT_RECOVERY","scopes":[],"procedure":{"sha256":"9595959595959595959595959595959595959595959595959595959595959595","schema_id":"rx.test.procedure.v1","size_bytes":"1"},"lead":"admin","operation_ids":[],"material_ids":[]}});
    let (status, case, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/open",
            Some(&cookie),
            body.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let before = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await
    .1;
    let cell = before["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["cell"]["value"]["id"] == "cell/a")
        .unwrap()["cell"]
        .clone();
    let prep = json!({"request_key":id(),"command":{"cell":"cell/a","expected_cell":cell["revision"],"case_revisions":[{"case":case["case"]["id"],"revision":case["revision"]}],"evidence_ids":[id()]}});
    let (status, denied, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/cases/close-preparations",
            Some(&cookie),
            prep.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(denied["code"], "CONDITION_UNKNOWN");
    assert_eq!(denied["outcome_unknown"], false);
    let mut malformed = prep.clone();
    malformed["command"]
        .as_object_mut()
        .unwrap()
        .remove("expected_cell");
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cases/close-preparations",
                Some(&cookie),
                malformed.to_string()
            )
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let reader = login(&f.app, "reader").await;
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cases/close-preparations",
                Some(&reader),
                prep.to_string()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let close = json!({"request_key":id(),"command":{"cell":"cell/a","expected_cell":cell["revision"],"case_revisions":prep["command"]["case_revisions"],"clearance":id()}});
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/cases/close-without-restart",
                Some(&cookie),
                close.to_string()
            )
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let after = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await
    .1;
    let cell_after = &after["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["cell"]["value"]["id"] == "cell/a")
        .unwrap()["cell"];
    assert_eq!(&cell, cell_after);
}

#[tokio::test]
async fn canonical_cell_inspection_uses_current_human_scope_and_frozen_json() {
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    let path = "/api/cell/v1/cells/cell%2Fa/inspect";
    assert_eq!(
        send(&f.app, request("GET", path, None, String::new()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, context, _) =
        send(&f.app, request("GET", path, Some(&cookie), String::new())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["cell_id"], "cell/a");
    assert_eq!(context["revision"], "1");
    assert_eq!(context["mode"], "SETUP");
    assert_eq!(context["commissioning"], "NOT_COMMISSIONED");
    assert_eq!(context["open_case_ids"].as_array().unwrap().len(), 0);
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                "/api/cell/v1/cells/cell%2Fb/inspect",
                Some(&cookie),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    f.handle
        .call(Command::Hold {
            identity: f.admin.clone(),
            key: id(),
            cell: name("cell/a"),
        })
        .await
        .unwrap();
    let (_, next, _) = send(&f.app, request("GET", path, Some(&cookie), String::new())).await;
    assert_eq!(next["blocks"][0]["kind"], "LATCHED");
    assert_eq!(next["blocks"][0]["created_revision"], "2");
    assert_eq!(next["commissioning"], "NOT_COMMISSIONED");
}

#[tokio::test]
async fn cell_wire_projection_rejects_missing_legacy_metadata_and_impossible_creation_revision() {
    let f = fixture().await;
    f.handle
        .call(Command::Hold {
            identity: f.admin.clone(),
            key: id(),
            cell: name("cell/a"),
        })
        .await
        .unwrap();
    let Reply::Cell(revision, cell) = f
        .handle
        .call(Command::InspectCell {
            identity: f.admin.clone(),
            cell: name("cell/a"),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let wire = rx_protocol_adapter::cell_context::view(revision, &cell).unwrap();
    assert_eq!(wire.blocks[0].created_revision, 2);
    for field in ["mode", "commissioning", "created_revision"] {
        let mut legacy = serde_json::to_value(&cell).unwrap();
        if field == "created_revision" {
            legacy["blocks"][0].as_object_mut().unwrap().remove(field);
        } else {
            legacy.as_object_mut().unwrap().remove(field);
        }
        let stored: Cell = serde_json::from_value(legacy).unwrap();
        let result = rx_protocol_adapter::cell_context::view(revision, &stored).unwrap_err();
        assert_eq!(result.code(), tonic::Code::FailedPrecondition);
        assert!(result.message().starts_with("UPGRADE_REQUIRED"));
    }
    let mut invalid = *cell;
    invalid.blocks[0].created_revision = Some(Counter(revision.0 + 1));
    assert_eq!(
        rx_protocol_adapter::cell_context::view(revision, &invalid)
            .unwrap_err()
            .code(),
        tonic::Code::DataLoss
    );
}

fn service_report(at: TimePoint) -> rx_application::service_health::Report {
    use rx_application::service_health::*;
    Report {
        connection: Connection::Bound,
        observation: Observation::Received,
        delivery: Delivery::Active,
        last_observation_at: Some(at.clone()),
        last_delivery_pass_at: Some(at),
        delivery_error_history: false,
    }
}
#[tokio::test]
async fn service_diagnostics_expire_without_rejuvenating_worker_activity_and_reject_old_owners() {
    use rx_application::service_health::*;
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    let target = Target {
        host: name("host/sim"),
        cell: name("cell/a"),
    };
    let Reply::HostServiceOwners(owners) = f
        .handle
        .call(Command::ConfigureHostServices(vec![target]))
        .await
        .unwrap()
    else {
        panic!("owners")
    };
    let owner = owners[0].clone();
    let (_, waiting, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(
        waiting["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["availability"],
        "WAITING_REPORT"
    );
    let initial_cell = waiting["cells"][0]["cell"].clone();
    let at = TimePoint {
        clock_id: "test-clock".into(),
        ticks_ns: Counter(1000),
    };
    let publish = Publish {
        owner: owner.clone(),
        sequence: Counter(1),
        report: service_report(at.clone()),
    };
    f.handle
        .call(Command::PublishHostService(Box::new(publish.clone())))
        .await
        .unwrap();
    assert!(
        f.handle
            .call(Command::PublishHostService(Box::new(publish)))
            .await
            .is_err()
    );
    let (_, fresh, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    let runtime = &fresh["cells"][0]["diagnostics"]["hosts"][0]["runtime"];
    assert_eq!(runtime["availability"], "FRESH");
    assert_eq!(runtime["observation_recent"], true);
    assert_eq!(runtime["delivery_recent"], true);
    assert_eq!(fresh["cells"][0]["cell"], initial_cell);
    assert!(!fresh.to_string().contains(owner.id.as_str()));
    f.clock.0.store(4_000_001_000, Ordering::SeqCst);
    let (_, stale, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(
        stale["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["availability"],
        "STALE"
    );
    f.handle
        .call(Command::PublishHostService(Box::new(Publish {
            owner: owner.clone(),
            sequence: Counter(2),
            report: service_report(at),
        })))
        .await
        .unwrap();
    let (_, heartbeat, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    let runtime = &heartbeat["cells"][0]["diagnostics"]["hosts"][0]["runtime"];
    assert_eq!(runtime["availability"], "FRESH");
    assert_eq!(runtime["observation_recent"], false);
    assert_eq!(runtime["delivery_recent"], false);
    let Reply::HostServiceOwner(new) = f
        .handle
        .call(Command::ReplaceHostService(owner.clone()))
        .await
        .unwrap()
    else {
        panic!("owner")
    };
    assert_ne!(new.id, owner.id);
    assert!(
        f.handle
            .call(Command::PublishHostService(Box::new(Publish {
                owner,
                sequence: Counter(99),
                report: service_report(f.clock.now())
            })))
            .await
            .is_err()
    );
    let (_, replaced, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(
        replaced["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["availability"],
        "WAITING_REPORT"
    );
    assert_eq!(replaced["cells"][0]["cell"], initial_cell);
    if let Ok(directory) = std::env::var("RX_SERVICE_HEALTH_FIXTURES_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir(&directory).unwrap();
        for (label, view) in [
            ("waiting", waiting),
            ("fresh", fresh),
            ("stale", stale),
            ("heartbeat", heartbeat),
            ("replaced", replaced),
        ] {
            std::fs::write(
                directory.join(format!("{label}.json")),
                rx_domain::canonical::bytes(&view).unwrap(),
            )
            .unwrap();
        }
    }
}
#[tokio::test]
async fn service_diagnostics_are_scope_filtered_and_new_runtime_cannot_reuse_a_previous_owner() {
    use rx_application::service_health::*;
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    let targets = vec![
        Target {
            host: name("host/sim"),
            cell: name("cell/a"),
        },
        Target {
            host: name("host/sim"),
            cell: name("cell/b"),
        },
    ];
    let Reply::HostServiceOwners(owners) = f
        .handle
        .call(Command::ConfigureHostServices(targets.clone()))
        .await
        .unwrap()
    else {
        panic!("owners")
    };
    let mut hidden = service_report(f.clock.now());
    hidden.delivery_error_history = true;
    f.handle
        .call(Command::PublishHostService(Box::new(Publish {
            owner: owners[1].clone(),
            sequence: Counter(1),
            report: hidden,
        })))
        .await
        .unwrap();
    let (_, view, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(view["cells"].as_array().unwrap().len(), 1);
    assert!(!view.to_string().contains("cell/b"));
    assert_eq!(
        view["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["delivery_error_history"],
        false
    );
    let mut reader = f.reader.clone();
    reader.active = false;
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: reader,
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        send(
            &f.app,
            request("GET", "/api/v1/overview", Some(&cookie), String::new())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let next = fixture().await;
    next.handle
        .call(Command::ConfigureHostServices(targets))
        .await
        .unwrap();
    assert!(
        next.handle
            .call(Command::PublishHostService(Box::new(Publish {
                owner: owners[0].clone(),
                sequence: Counter(3),
                report: service_report(next.clock.now())
            })))
            .await
            .is_err()
    );
    let cookie = login(&next.app, "reader").await;
    let (_, view, _) = send(
        &next.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(
        view["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["availability"],
        "WAITING_REPORT"
    );
}

#[tokio::test]
async fn invalid_service_inventory_is_not_partially_configured_and_empty_inventory_is_explicit() {
    use rx_application::service_health::Target;
    let f = fixture().await;
    let cookie = login(&f.app, "reader").await;
    let target = Target {
        host: name("host/sim"),
        cell: name("cell/a"),
    };
    assert!(
        f.handle
            .call(Command::ConfigureHostServices(vec![
                target.clone(),
                Target {
                    host: name("missing-host"),
                    cell: name("cell/a")
                }
            ]))
            .await
            .is_err()
    );
    assert!(
        f.handle
            .call(Command::ConfigureHostServices(vec![
                target.clone(),
                target.clone()
            ]))
            .await
            .is_err()
    );
    let (_, view, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert!(view["cells"][0]["diagnostics"]["hosts"][0]["runtime"].is_null());
    f.handle
        .call(Command::ConfigureHostServices(vec![]))
        .await
        .unwrap();
    let (_, view, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(
        view["cells"][0]["diagnostics"]["hosts"][0]["runtime"]["availability"],
        "NOT_CONFIGURED"
    );
    assert!(
        f.handle
            .call(Command::ConfigureHostServices(vec![target]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn process_draft_http_preserves_versions_and_blocks_non_engineer_or_cross_cell_writes() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let reader = login(&f.app, "reader").await;
    let draft = id();
    let key = id();
    let document = json!({"schema":"rx.process-source.v1","process":"example/draft","entry":"main","conditions":{},"flows":[{"id":"main","root":"sequence","nodes":[{"id":"sequence","body":{"kind":"SEQUENCE","children":[]}}]}]});
    let command = json!({"id":draft,"cell":"cell/a","expected":null,"title":"First draft","document":document});
    let body = json!({"request_key":key,"command":command}).to_string();
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-drafts",
                Some(&reader),
                body.clone()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, first, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-drafts",
            Some(&cookie),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["version"]["revision"], "1");
    assert_eq!(first["version"]["validation"]["structurally_valid"], false);
    let (_, again, _) = send(
        &f.app,
        request("POST", "/api/v1/process-drafts", Some(&cookie), body),
    )
    .await;
    assert_eq!(again, first);
    let mut next = command;
    next["expected"] = json!("1");
    next["title"] = json!("Second draft");
    let (status, second, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-drafts",
            Some(&cookie),
            json!({"request_key":id(),"command":next}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["version"]["revision"], "2");
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-drafts",
                Some(&cookie),
                json!({"request_key":id(),"command":next}).to_string()
            )
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let path = format!("/api/v1/process-draft?cell=cell%2Fa&id={draft}&revision=1");
    let (status, version, _) =
        send(&f.app, request("GET", &path, Some(&cookie), String::new())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(version, first);
    assert_eq!(
        send(&f.app, request("GET", &path, Some(&reader), String::new()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                &format!("/api/v1/process-draft?cell=cell%2Fb&id={draft}"),
                Some(&cookie),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, page, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/process-drafts?cell=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(page["drafts"].as_array().unwrap().len(), 1);
    assert!(page["drafts"][0].get("document").is_none());
    let (_, overview, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(overview["cells"][0]["cell"]["revision"], "1");
    assert!(overview["cells"][0]["runs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn binding_http_produces_a_matched_compile_input_without_cell_activation() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let reader = login(&f.app, "reader").await;
    let draft = id();
    let document = json!({"schema":"rx.process-source.v1","process":"http/binding","entry":"main","conditions":{},"flows":[{"id":"main","root":"work","nodes":[{"id":"work","body":{"kind":"OPERATION","binding":"work"}}]}]});
    let(status,_,_)=send(&f.app,request("POST","/api/v1/process-drafts",Some(&cookie),json!({"request_key":id(),"command":{"id":draft,"cell":"cell/a","expected":null,"title":"bound draft","document":document}}).to_string())).await;
    assert_eq!(status, StatusCode::OK);
    let (status, catalog, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/process-draft/binding-options?cell=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body=json!({"request_key":id(),"command":{"draft":draft,"cell":"cell/a","source_revision":"1","expected":null,"catalog_digest":catalog["catalog_digest"],"selections":{"work":catalog["candidates"][0]["step"]}}}).to_string();
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-draft-bindings",
                Some(&reader),
                body.clone()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, saved, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-draft-bindings",
            Some(&cookie),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["complete"], true);
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-draft-bindings",
                Some(&cookie),
                body
            )
        )
        .await
        .1,
        saved
    );
    let path = format!(
        "/api/v1/process-draft-compile-input?cell=cell%2Fa&id={draft}&source_revision=1&binding_revision=1"
    );
    let (status, bundle, _) =
        send(&f.app, request("GET", &path, Some(&cookie), String::new())).await;
    assert_eq!(status, StatusCode::OK);
    let input: rx_process_contract::compile_input::CompileInput =
        serde_json::from_value(bundle).unwrap();
    assert!(input.validate().is_ok());
    assert_eq!(
        input.bindings[&name("work")].intent.digest().unwrap(),
        f.configuration.steps[0].intent.digest().unwrap()
    );
    assert_eq!(
        send(&f.app, request("GET", &path, Some(&reader), String::new()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, view, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert!(view["cells"][0]["cell"]["value"]["qualification"].is_null());
    assert_eq!(view["cells"][0]["cell"]["revision"], "1");
}

#[path = "../../rx-application/tests/support/package_intake.rs"]
mod intake_support;
#[tokio::test]
async fn package_intake_http_verifies_real_files_scopes_receipts_and_does_not_activate() {
    let mut f = fixture().await;
    let p = intake_support::fixture();
    let object = p.object.clone();
    let source = p.import_root.join("test-package/process.json");
    let worker = rx_runtime::package_intake::Worker::new(
        p.import_root,
        p.store,
        p.policy,
        rx_package::content_digest(&p.policy_bytes),
    )
    .unwrap();
    f.handle
        .call(Command::ConfigurePackageIntake(Some(worker.registration())))
        .await
        .unwrap();
    f.app = rx_api::router_with_package_intake(
        Arc::new(f.handle.clone()),
        credentials(),
        LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
        worker,
    )
    .unwrap();
    let cookie = login(&f.app, "admin").await;
    let reader = login(&f.app, "reader").await;
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                "/api/v1/package-intake-context?cell=cell%2Fa",
                Some(&reader),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, context, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/package-intake-context?cell=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let package_id = id();
    let key = id();
    let command = json!({"id":package_id,"cell":"cell/a","title":"Machine-tending package","relative_path":"test-package","object":object,"configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"]});
    let body = json!({"request_key":key,"command":command}).to_string();
    let (status, receipt, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/package-intakes",
            Some(&cookie),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["state"], "AWAITING_REVIEW");
    std::fs::write(&source, b"corrupt after admission").unwrap();
    let (status, recovered, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/package-intakes",
            Some(&cookie),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered, receipt);
    let mut new = command.clone();
    new["id"] = json!(id());
    let (status, failed, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/package-intakes",
            Some(&cookie),
            json!({"request_key":id(),"command":new}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(failed["code"], "PACKAGE_VERIFICATION_FAILED");
    let (status, page, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/package-intakes?cell=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["packages"].as_array().unwrap().len(), 1);
    assert_eq!(page["packages"][0]["activation_authorized"], false);
    assert_eq!(page["packages"][0]["content_reverification_required"], true);
    assert_eq!(
        send(
            &f.app,
            request(
                "GET",
                &format!("/api/v1/package-intake?cell=cell%2Fb&id={package_id}"),
                Some(&cookie),
                String::new()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut forged = command;
    forged["approved"] = json!(true);
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/package-intakes",
                Some(&cookie),
                json!({"request_key":id(),"command":forged}).to_string()
            )
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    f.handle
        .call(Command::ConfigurePackageIntake(None))
        .await
        .unwrap();
    let (_, view, _) = send(
        &f.app,
        request(
            "GET",
            &format!("/api/v1/package-intake?cell=cell%2Fa&id={package_id}"),
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(view["review_context_current"], false);
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: principal(
                "admin",
                &[Role::AccountAdmin, Role::Observer],
                &["cell/a", "cell/b", "cell/c"],
            ),
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        send(
            &f.app,
            request("POST", "/api/v1/package-intakes", Some(&cookie), body)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let Reply::Overview(overview) = f
        .handle
        .call(Command::Overview(f.admin.clone()))
        .await
        .unwrap()
    else {
        panic!("overview")
    };
    assert!(overview.cells.iter().all(|c| c.runs.is_empty()));
    f.handle.close();
    let _ = f.handle.closed().await;
}

#[path = "../../rx-application/tests/support/process_review.rs"]
mod review_support;
#[tokio::test]
#[ignore = "tools/test_process_review_e2e.sh supplies the separately built S verifier"]
async fn process_review_real_compiler_report_crosses_http_and_approval_rechecks_store() {
    let tool =
        std::path::PathBuf::from(std::env::var("RX_PROCESS_PACKAGE_BIN").expect("S verifier path"));
    let output = std::process::Command::new(&tool)
        .arg("validator-identity")
        .output()
        .unwrap();
    assert!(output.status.success());
    let identity: Value = serde_json::from_slice(&output.stdout).unwrap();
    let validator: Digest = serde_json::from_value(identity["validator_digest"].clone()).unwrap();
    let mut f = fixture().await;
    let p = review_support::fixture(&f.configuration, validator);
    let object = p.object.clone();
    let root = p._dir.path().to_path_buf();
    let authority_path = root.join("authority.json");
    let authority_bytes = rx_domain::canonical::bytes(&p.authority).unwrap();
    std::fs::write(&authority_path, &authority_bytes).unwrap();
    let worker = rx_runtime::package_intake::Worker::new_pinned(
        root.clone(),
        p.store,
        p.policy,
        rx_package::content_digest(&p.policy_bytes),
        p.policy_path.clone(),
    )
    .unwrap();
    let worker = rx_runtime::package_intake::Worker::with_review(
        worker,
        authority_path,
        rx_package::content_digest(&authority_bytes),
    )
    .unwrap();
    f.handle
        .call(Command::ConfigurePackageIntake(Some(worker.registration())))
        .await
        .unwrap();
    f.handle
        .call(Command::ConfigureProcessReview(worker.review_digest()))
        .await
        .unwrap();
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: principal("reviewer", &[Role::Verifier], &["cell/a"]),
            expected: None,
        })
        .await
        .unwrap();
    let mut credentials = credentials();
    credentials.accounts.push(LocalAccount {
        principal: name("reviewer"),
        password_hash: credentials.accounts[0].password_hash.clone(),
    });
    f.app = rx_api::router_with_package_intake(
        Arc::new(f.handle.clone()),
        credentials,
        LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
        worker,
    )
    .unwrap();
    let engineer = login(&f.app, "admin").await;
    let reviewer = login(&f.app, "reviewer").await;
    let (_, context, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/package-intake-context?cell=cell%2Fa",
            Some(&engineer),
            String::new(),
        ),
    )
    .await;
    let intake = id();
    let (status,receipt,_)=send(&f.app,request("POST","/api/v1/package-intakes",Some(&engineer),json!({"request_key":id(),"command":{"id":intake,"cell":"cell/a","title":"Compiler review integration","relative_path":"package","object":object,"configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"]}}).to_string())).await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    let review = id();
    let (status,job,_)=send(&f.app,request("POST","/api/v1/process-reviews",Some(&engineer),json!({"request_key":id(),"command":{"id":review,"intake":intake,"cell":"cell/a","configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"],"binding_selections":{"load":f.configuration.steps[0].id}}}).to_string())).await;
    assert_eq!(status, StatusCode::OK, "{job}");
    let request_path = root.join("request.json");
    std::fs::write(
        &request_path,
        rx_domain::canonical::bytes(&job["request"]).unwrap(),
    )
    .unwrap();
    let report_dir = root.join("report");
    let output = std::process::Command::new(&tool)
        .arg("review")
        .arg(&p.package)
        .arg(&p.policy_path)
        .arg(&request_path)
        .arg(&report_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compiler: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(compiler["compiler_checks_passed"], true, "{compiler}");
    let report: rx_process_contract::package_review::Report = rx_domain::canonical::decode_json(
        &std::fs::read(report_dir.join("verification.json")).unwrap(),
    )
    .unwrap();
    let signature = review_support::sign(&report);
    std::fs::write(
        report_dir.join("verification.sig.json"),
        rx_domain::canonical::bytes(&signature).unwrap(),
    )
    .unwrap();
    let compiled: rx_process_contract::ResolvedProcess = rx_domain::canonical::decode_json(
        &std::fs::read(report_dir.join("resolved.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rx_domain::canonical::bytes(&compiled).unwrap(),
        rx_domain::canonical::bytes(&p.resolved).unwrap()
    );
    assert_eq!(report.validator_digest, p.validator);
    let (status,version,_)=send(&f.app,request("POST","/api/v1/process-review/reports",Some(&engineer),json!({"request_key":id(),"command":{"review":review,"cell":"cell/a","expected":null,"directory":"report","report_digest":report.digest().unwrap()}}).to_string())).await;
    assert_eq!(status, StatusCode::OK, "{version}");
    assert_eq!(version["ready_for_software_approval"], true);
    let decision = json!({"request_key":id(),"command":{"review":review,"cell":"cell/a","report_revision":version["revision"],"review_digest":version["review_digest"],"expected":null,"choice":"APPROVE","note":"Reviewed source, compiler output and current cell bindings"}});
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-review/decisions",
                Some(&engineer),
                decision.to_string()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let stored_source = root
        .join("store")
        .join(format!("{}-{}", object.manifest, object.signature))
        .join("process/source.json");
    let original = std::fs::read(&stored_source).unwrap();
    std::fs::write(&stored_source, b"{}").unwrap();
    let (status, failed, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-review/decisions",
            Some(&reviewer),
            decision.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(failed["code"], "REVIEW_REVERIFICATION_FAILED");
    std::fs::write(&stored_source, original).unwrap();
    let (status, approved, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-review/decisions",
            Some(&reviewer),
            decision.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["choice"], "APPROVE");
    assert_eq!(approved["scope"], "PROCESS_PACKAGE_SOFTWARE");
    let (_, repeated, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-review/decisions",
            Some(&reviewer),
            decision.to_string(),
        ),
    )
    .await;
    assert_eq!(approved, repeated);
    let (_, view, _) = send(
        &f.app,
        request(
            "GET",
            &format!("/api/v1/process-review?cell=cell%2Fa&id={review}"),
            Some(&reviewer),
            String::new(),
        ),
    )
    .await;
    assert_eq!(view["approval_matches_current_review"], true);
    assert_eq!(view["activation_authorized"], false);
    assert!(view["source"].is_object() && view["resolved"].is_object());
    let (_, overview, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&engineer), String::new()),
    )
    .await;
    assert!(
        overview["cells"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["runs"].as_array().unwrap().is_empty()
                && c["cell"]["value"]["qualification"].is_null())
    );
    f.handle
        .call(Command::PutPrincipal {
            identity: f.admin.clone(),
            value: principal(
                "reviewer",
                &[Role::Verifier, Role::ReleaseManager],
                &["cell/a", "cell/b"],
            ),
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    let change_id = id();
    let (status,proposed,_)=send(&f.app,request("POST","/api/v1/process-changes",Some(&engineer),json!({"request_key":id(),"command":{"id":change_id,"cell":"cell/a","review":{"id":review,"revision":version["revision"],"review_digest":version["review_digest"],"decision_revision":approved["revision"]},"reason":"Stage approved software with explicit whole-cell impact review"}}).to_string())).await;
    assert_eq!(status, StatusCode::OK, "{proposed}");
    assert_eq!(proposed["state"], "PROPOSED");
    assert_eq!(proposed["impact"]["cells"].as_array().unwrap().len(), 2);
    let target = |c: &Value| json!({"change":change_id,"cell":"cell/a","expected":c["revision"],"plan_digest":c["plan_digest"]});
    let (status,impact,_)=send(&f.app,request("POST","/api/v1/process-change/impact-review",Some(&reviewer),json!({"request_key":id(),"command":{"target":target(&proposed),"note":"Shared Host/resource scope, qualification and recovery effects reviewed"}}).to_string())).await;
    assert_eq!(status, StatusCode::OK, "{impact}");
    let (status, staged, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-change/stage",
            Some(&reviewer),
            json!({"request_key":id(),"command":target(&impact)}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    assert_eq!(staged["state"], "STAGED");
    assert_eq!(
        send(
            &f.app,
            request(
                "POST",
                "/api/v1/process-change/prepare",
                Some(&reviewer),
                json!({"request_key":id(),"command":{"target":target(&staged),"refresh":false}})
                    .to_string()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, change_view, _) = send(
        &f.app,
        request(
            "GET",
            &format!("/api/v1/process-change?cell=cell%2Fa&id={change_id}"),
            Some(&reviewer),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(change_view["applied"], false);
    assert_eq!(change_view["activation_authorized"], false);
    assert_eq!(
        change_view["after"]["process"],
        serde_json::to_value(&compiled).unwrap()
    );
    if let Ok(directory) = std::env::var("RX_REVIEW_EVIDENCE_DIR") {
        let out = std::path::PathBuf::from(directory);
        std::fs::create_dir(&out).unwrap();
        for (label, value) in [
            ("compiler.json", compiler),
            ("job.json", job),
            ("verification-version.json", version),
            ("decision.json", approved),
            ("view.json", view),
            ("staged-change.json", change_view),
        ] {
            std::fs::write(
                out.join(label),
                rx_domain::canonical::bytes(&value).unwrap(),
            )
            .unwrap();
        }
        std::fs::write(
            out.join("signature.json"),
            rx_domain::canonical::bytes(&signature).unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join("report.json"),
            rx_domain::canonical::bytes(&report).unwrap(),
        )
        .unwrap();
        std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&json!({"schema":"rx.process-review-e2e.v1","status":"PASS","real_s_compiler":true,"trusted_test_signer_only":true,"source_corruption_blocks_approval":true,"separate_reviewer":true,"same_key_recovers_decision":true,"activation_authorized":false})).unwrap()).unwrap();
    }
    f.handle.close();
    let _ = f.handle.closed().await;
}

#[test]
#[ignore = "browser package harness requests isolated fixture files; no production keys"]
fn export_operator_package_fixture() {
    use rx_ports::Repository;
    let out = std::path::PathBuf::from(std::env::var("RX_PACKAGE_BROWSER_FIXTURE").unwrap());
    std::fs::create_dir(&out).unwrap();
    let binary = std::env::var("RX_PROCESS_PACKAGE_BIN").unwrap();
    let output = std::process::Command::new(binary)
        .arg("validator-identity")
        .output()
        .unwrap();
    assert!(output.status.success());
    let identity: Value = serde_json::from_slice(&output.stdout).unwrap();
    let validator: Digest = serde_json::from_value(identity["validator_digest"].clone()).unwrap();
    let state = out.join("installation");
    std::fs::create_dir(&state).unwrap();
    let exchange = out.join("exchange");
    std::fs::create_dir(&exchange).unwrap();
    let cfg = configuration("cell/demo");
    let package = review_support::fixture(&cfg, validator);
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy_tree(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy_tree(&package.package, &exchange.join("package"));
    fn private(path: &std::path::Path, value: &impl serde::Serialize) {
        std::fs::write(path, rx_domain::canonical::bytes(value).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    let policy_path = state.join("package-policy.json");
    std::fs::write(&policy_path, &package.policy_bytes).unwrap();
    let authority_path = state.join("review-authority.json");
    private(&authority_path, &package.authority);
    let mut admin = principal(
        "admin",
        &[
            Role::AccountAdmin,
            Role::Engineer,
            Role::Verifier,
            Role::Observer,
        ],
        &["cell/demo", "cell/other"],
    );
    if std::env::var("RX_HOST_BINDING_PLAN_ENABLED").as_deref() == Ok("1") {
        admin.roles.insert(Role::ReleaseManager);
    }
    let installation = id();
    let mut engine = Engine::open(
        SqliteRepository::open(state.join("platform.db")).unwrap(),
        ClockSource(Arc::new(AtomicU64::new(1000))),
        Deny,
        installation.clone(),
        admin.clone(),
    )
    .unwrap();
    let session = engine
        .authenticated_session(
            &admin.id,
            id(),
            TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(100000),
            },
        )
        .unwrap();
    let who = Identity {
        principal: admin.id.clone(),
        session: session.id,
        terminal: None,
    };
    engine
        .put_principal(
            &who,
            principal("reviewer", &[Role::Verifier], &["cell/demo"]),
            None,
        )
        .unwrap();
    engine
        .put_principal(
            &who,
            principal("reader", &[Role::Observer], &["cell/demo"]),
            None,
        )
        .unwrap();
    if std::env::var("RX_DEVICE_BINDING_ENABLED").as_deref() == Ok("1") {
        engine
            .put_principal(
                &who,
                principal(
                    "impact-reviewer",
                    &[Role::Verifier],
                    &["cell/demo", "cell/other"],
                ),
                None,
            )
            .unwrap();
    }
    engine.install_cell(&who, cfg).unwrap();
    let mut other = configuration("cell/other");
    if std::env::var("RX_HOST_BINDING_PLAN_ENABLED").as_deref() == Ok("1") {
        other.hosts = vec![name("host/other")];
        for step in &mut other.steps {
            step.host = name("host/other");
        }
        for fact in &mut other.fact_specs {
            fact.host = name("host/other");
        }
    }
    engine.install_cell(&who, other).unwrap();
    let mut repository = engine.into_repository();
    repository.transact(|_| Ok(())).unwrap();
    drop(repository);
    private(
        &state.join("installation.json"),
        &json!({"schema":"rx.development-installation.v1","installation_id":installation,"bootstrap":admin}),
    );
    let password = password_hash("browser-fixture-password").unwrap();
    private(
        &state.join("credentials.json"),
        &Credentials {
            schema: "rx.local-credentials.v1".into(),
            accounts: ["admin", "reviewer", "reader"]
                .into_iter()
                .chain(
                    (std::env::var("RX_DEVICE_BINDING_ENABLED").as_deref() == Ok("1"))
                        .then_some("impact-reviewer"),
                )
                .map(|p| LocalAccount {
                    principal: name(p),
                    password_hash: password.clone(),
                })
                .collect(),
        },
    );
    private(
        &state.join("package-service.json"),
        &json!({"schema":"rx.development-package-service.v1","import_root":exchange,"policy":{"path":policy_path,"sha256":rx_package::content_digest(&package.policy_bytes)},"review_authority":{"path":authority_path,"sha256":rx_package::content_digest(&std::fs::read(&authority_path).unwrap())}}),
    );
    std::fs::write(out.join("fixture.json"),serde_json::to_vec_pretty(&json!({"object":package.object,"exchange":exchange,"policy":policy_path,"installation":state,"site_config_digest":configuration("cell/demo").site_config_digest})).unwrap()).unwrap();
}
#[test]
#[ignore = "test-only signing for browser-generated process review requests"]
fn sign_operator_review_fixture() {
    let report_path = std::path::PathBuf::from(std::env::var("RX_BROWSER_REVIEW_REPORT").unwrap());
    let report: rx_process_contract::package_review::Report =
        rx_domain::canonical::decode_json(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report.request.cell.as_str(), "cell/demo");
    let signature = review_support::sign(&report);
    std::fs::write(
        report_path.parent().unwrap().join("verification.sig.json"),
        rx_domain::canonical::bytes(&signature).unwrap(),
    )
    .unwrap();
}
