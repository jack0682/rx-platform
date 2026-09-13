mod device_binding;
mod device_review;
mod host_recovery;
mod investigation;
use crate::{
    auth::{Auth, COOKIE, Credentials, SESSION_SECONDS},
    error::ApiError,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rx_application::{CellConfiguration, CreateRun, Identity, StartRun};
use rx_domain::{canonical, types::*};
use rx_runtime::{
    application::{ApplicationPort, Command, Reply},
    writer::Status,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{net::SocketAddr, sync::Arc};

/// Deliberately supports loopback development only. LAN requires a separate authenticated TLS ingress.
#[derive(Clone)]
pub struct LocalPolicy {
    origin: String,
    host: String,
}
impl LocalPolicy {
    pub fn new(origin: &str) -> Result<Self, String> {
        let uri: Uri = origin.parse().map_err(|_| "invalid public origin")?;
        if uri.scheme_str() != Some("http")
            || !matches!(uri.host(), Some("127.0.0.1" | "[::1]" | "localhost"))
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            || origin.ends_with('/')
        {
            return Err(
                "local service requires an exact HTTP loopback origin without trailing slash"
                    .into(),
            );
        }
        Ok(Self {
            origin: origin.into(),
            host: uri.authority().ok_or("origin authority")?.to_string(),
        })
    }
}
#[derive(Clone)]
struct ApiState {
    runtime: Arc<dyn ApplicationPort>,
    auth: Arc<Auth>,
    policy: LocalPolicy,
    terminal_tls: bool,
    package_intake: Option<Arc<rx_runtime::package_intake::Worker>>,
    host_recovery: Option<Arc<dyn rx_runtime::host_recovery::Service>>,
    investigation: Option<Arc<rx_runtime::investigation::Worker>>,
}

pub fn router(
    runtime: Arc<dyn ApplicationPort>,
    credentials: Credentials,
    policy: LocalPolicy,
) -> Result<Router, String> {
    build_router(runtime, credentials, policy, false, None, None, None)
}
pub fn router_with_package_intake(
    runtime: Arc<dyn ApplicationPort>,
    credentials: Credentials,
    policy: LocalPolicy,
    worker: Arc<rx_runtime::package_intake::Worker>,
) -> Result<Router, String> {
    build_router(
        runtime,
        credentials,
        policy,
        false,
        Some(worker),
        None,
        None,
    )
}
pub(crate) fn terminal_router(
    runtime: Arc<dyn ApplicationPort>,
    credentials: Credentials,
    policy: crate::terminal_https::HttpsPolicy,
    worker: Option<Arc<rx_runtime::package_intake::Worker>>,
    recovery: Option<Arc<dyn rx_runtime::host_recovery::Service>>,
    investigation: Option<Arc<rx_runtime::investigation::Worker>>,
) -> Result<Router, String> {
    build_router(
        runtime,
        credentials,
        LocalPolicy {
            origin: policy.origin,
            host: policy.host,
        },
        true,
        worker,
        recovery,
        investigation,
    )
}
fn build_router(
    runtime: Arc<dyn ApplicationPort>,
    credentials: Credentials,
    policy: LocalPolicy,
    terminal_tls: bool,
    package_intake: Option<Arc<rx_runtime::package_intake::Worker>>,
    host_recovery: Option<Arc<dyn rx_runtime::host_recovery::Service>>,
    investigation: Option<Arc<rx_runtime::investigation::Worker>>,
) -> Result<Router, String> {
    let state = ApiState {
        runtime,
        auth: Arc::new(Auth::new(credentials)?),
        policy,
        terminal_tls,
        package_intake,
        host_recovery,
        investigation,
    };
    Ok(Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/session", post(login).get(profile))
        .route("/api/v1/session/end", post(logout))
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/investigation-context", get(investigation::context))
        .route(
            "/api/v1/investigation-attestations",
            get(investigation::list_attestations).post(investigation::attest),
        )
        .route(
            "/api/v1/investigation-attestation",
            get(investigation::get_attestation),
        )
        .route(
            "/api/v1/recovery-dispositions",
            get(investigation::list_dispositions).post(investigation::record_disposition),
        )
        .route(
            "/api/v1/recovery-disposition",
            get(investigation::get_disposition),
        )
        .route("/api/v1/host-recovery-context", get(host_recovery::context))
        .route(
            "/api/v1/host-recoveries",
            get(host_recovery::list).post(host_recovery::propose),
        )
        .route("/api/v1/host-recovery", get(host_recovery::get))
        .route(
            "/api/v1/host-recovery/approve",
            post(host_recovery::approve),
        )
        .route(
            "/api/v1/host-recovery/progress",
            post(host_recovery::progress),
        )
        .route("/api/v1/host-recovery/query", post(host_recovery::query))
        .route(
            "/api/v1/package-intake-context",
            get(package_intake_context),
        )
        .route(
            "/api/v1/package-intakes",
            get(package_intakes).post(submit_package_intake),
        )
        .route(
            "/api/v1/qualification-activations",
            post(issue_qualification),
        )
        .route(
            "/api/v1/qualification-activation",
            get(get_qualification_batch),
        )
        .route(
            "/api/v1/qualification-activation/activate",
            post(activate_qualification),
        )
        .route(
            "/api/v1/qualification-activation/suspend",
            post(suspend_qualification),
        )
        .route("/api/v1/qualification-reviews", post(begin_requalification))
        .route("/api/v1/qualification-review", get(get_requalification))
        .route(
            "/api/v1/qualification-review/reports",
            post(submit_requalification),
        )
        .route(
            "/api/v1/qualification-review/decisions",
            post(decide_requalification),
        )
        .route(
            "/api/v1/qualification-review/artifact",
            post(requalification_artifact),
        )
        .route("/api/v1/package-intake", get(get_package_intake))
        .route(
            "/api/v1/device-binding-plans",
            get(device_binding::list).post(device_binding::propose),
        )
        .route("/api/v1/device-binding-plan", get(device_binding::get))
        .route(
            "/api/v1/device-binding-plan/impact-review",
            post(device_binding::review),
        )
        .route(
            "/api/v1/device-reviews",
            get(device_review::list).post(device_review::create),
        )
        .route("/api/v1/device-review", get(device_review::get))
        .route("/api/v1/device-review/reports", post(device_review::report))
        .route(
            "/api/v1/device-review/decisions",
            post(device_review::decide),
        )
        .route(
            "/api/v1/package-intake/device-catalog",
            get(get_package_device_catalog),
        )
        .route(
            "/api/v1/process-reviews",
            get(list_process_reviews).post(create_process_review),
        )
        .route("/api/v1/process-review", get(get_process_review))
        .route("/api/v1/process-changes", post(propose_process_change))
        .route("/api/v1/process-change", get(get_process_change))
        .route("/api/v1/process-change/apply", post(apply_process_change))
        .route(
            "/api/v1/process-change/configure-hosts",
            post(authorize_host_configuration),
        )
        .route(
            "/api/v1/process-change/impact-review",
            post(review_change_impact),
        )
        .route("/api/v1/process-change/stage", post(stage_process_change))
        .route(
            "/api/v1/process-change/prepare",
            post(begin_change_preparation),
        )
        .route("/api/v1/process-review/reports", post(submit_review_report))
        .route(
            "/api/v1/process-review/decisions",
            post(decide_process_review),
        )
        .route(
            "/api/v1/process-drafts",
            get(process_drafts).post(save_process_draft),
        )
        .route("/api/v1/process-draft", get(process_draft))
        .route(
            "/api/v1/process-draft-compile-input",
            get(draft_compile_input),
        )
        .route(
            "/api/v1/process-draft/binding-options",
            get(draft_binding_catalog).post(draft_device_binding_catalog),
        )
        .route(
            "/api/v1/process-draft-bindings",
            get(draft_bindings).post(save_draft_bindings),
        )
        .route("/api/v1/cell", get(cell))
        .route("/api/v1/runtime-restrictions", get(runtime_restrictions))
        .route("/api/cell/v1/cells/{cell_id}/inspect", get(cell_context))
        .route("/api/v1/cells", post(install_cell))
        .route("/api/v1/runs", post(create_run))
        .route("/api/v1/run/start-context", get(operator_start_context))
        .route("/api/v1/run/start-attempt", get(operator_start_attempt))
        .route("/api/v1/run/checkpoint", get(run_checkpoint))
        .route("/api/v1/run/checkpoint/artifact", get(checkpoint_artifact))
        .route("/api/v1/runs/start", post(start_run))
        .route("/api/v1/cells/hold", post(hold))
        .route("/api/v1/cases", get(cases))
        .route("/api/v1/case", get(case_detail))
        .route("/api/v1/cases/open", post(open_case))
        .route("/api/v1/cases/acknowledge", post(acknowledge_case))
        .route("/api/v1/cases/procedure", post(record_procedure))
        .route("/api/v1/cases/close-preparations", post(prepare_close))
        .route(
            "/api/v1/cases/close-without-restart",
            post(close_without_restart),
        )
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND") })
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), ingress))
        .with_state(state))
}

async fn ingress(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let result = validate_ingress(&state, &request);
    let mut response = match result {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}
pub(crate) fn exactly_one<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(first)
    }
}
fn validate_ingress(state: &ApiState, request: &Request) -> Result<(), ApiError> {
    let policy = &state.policy;
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .ok_or_else(ApiError::forbidden)?;
    let terminal_peer = request
        .extensions()
        .get::<crate::terminal_https::TerminalPeer>();
    if (state.terminal_tls && terminal_peer.is_none())
        || (!state.terminal_tls && (!peer.0.ip().is_loopback() || terminal_peer.is_some()))
        || exactly_one(request.headers(), "host") != Some(&policy.host)
    {
        return Err(ApiError::forbidden());
    }
    let headers = request.headers();
    let login = request.method() == Method::POST && request.uri().path() == "/api/v1/session";
    if !login && request.uri().path() != "/api/v1/health" && headers.contains_key(header::COOKIE) {
        let identity = state.auth.resolve(cookie(headers)?)?;
        if identity
            .terminal
            .as_ref()
            .map(|(_, fingerprint)| *fingerprint)
            != terminal_peer.map(|p| p.fingerprint())
        {
            return Err(ApiError::unauthenticated());
        }
    }
    if headers.contains_key("origin") && exactly_one(headers, "origin") != Some(&policy.origin) {
        return Err(ApiError::forbidden());
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        if exactly_one(headers, "origin") != Some(&policy.origin)
            || exactly_one(headers, "x-rx-client") != Some("browser-v1")
        {
            return Err(ApiError::forbidden());
        }
        if exactly_one(headers, "content-type")
            .is_none_or(|v| !matches!(v, "application/json" | "application/json; charset=utf-8"))
        {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "JSON_REQUIRED",
            ));
        }
    }
    Ok(())
}
fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    canonical::decode_json(body).map_err(|_| ApiError::invalid())
}
fn cookie(headers: &HeaderMap) -> Result<&str, ApiError> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE).iter() {
        for segment in value
            .to_str()
            .map_err(|_| ApiError::unauthenticated())?
            .split(';')
        {
            let Some((key, value)) = segment.trim().split_once('=') else {
                continue;
            };
            if key == COOKIE {
                if found.is_some()
                    || value.len() != 64
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(ApiError::unauthenticated());
                }
                found = Some(value);
            }
        }
    }
    found.ok_or_else(ApiError::unauthenticated)
}
fn identity(state: &ApiState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    state.auth.resolve(cookie(headers)?)
}
fn mismatch() -> ApiError {
    ApiError::unavailable()
}

async fn health(State(s): State<ApiState>) -> Response {
    let writer_available = s.runtime.status() == Status::Running;
    let serving = matches!(s.runtime.request(Command::RuntimeLifecycle).await, Ok(Reply::RuntimeLifecycle(value)) if value.phase == rx_application::lifecycle::Phase::Serving);
    let ready = writer_available && serving;
    if s.terminal_tls {
        return (if ready {StatusCode::OK} else {StatusCode::SERVICE_UNAVAILABLE}, Json(serde_json::json!({"service":"rx-platform", "mode":"TERMINAL_HTTPS", "writer_available":writer_available, "admission_open":serving, "start_requires_user_terminal_and_runtime_conditions":true}))).into_response();
    }
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "service":"rx-platform", "mode":"DEVELOPMENT_LOOPBACK", "writer_available":writer_available, "admission_open":serving,
            "device_control_enabled":false, "terminal_authenticated":false
        })),
    )
        .into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    principal: String,
    password: String,
}
async fn login(
    State(s): State<ApiState>,
    peer: Option<Extension<crate::terminal_https::TerminalPeer>>,
    body: Bytes,
) -> Result<Response, ApiError> {
    if body.len() > 4096 {
        return Err(ApiError::invalid());
    }
    let value: Login = decode(&body)?;
    let principal = s.auth.verify(value.principal, value.password).await?;
    let id = Id::new(uuid::Uuid::new_v4().to_string()).map_err(|_| mismatch())?;
    let command = match peer {
        Some(Extension(peer)) if s.terminal_tls => Command::OpenTerminalUserSession {
            principal: principal.clone(),
            id,
            ttl_ns: Counter(SESSION_SECONDS * 1_000_000_000),
            certificate: peer.fingerprint(),
        },
        None if !s.terminal_tls => Command::OpenUserSession {
            principal: principal.clone(),
            id,
            ttl_ns: Counter(SESSION_SECONDS * 1_000_000_000),
        },
        _ => return Err(ApiError::forbidden()),
    };
    let reply = s.runtime.request(command).await?;
    let Reply::Session(session) = reply else {
        return Err(mismatch());
    };
    // The writer bound this verified connection to the current terminal registration.
    let identity = Identity {
        principal,
        session: session.id,
        terminal: session.terminal.map(|t| (t.id, t.certificate_digest)),
    };
    let Reply::Profile(profile) = s
        .runtime
        .request(Command::UserProfile(identity.clone()))
        .await?
    else {
        return Err(mismatch());
    };
    let token = s.auth.insert(identity)?;
    let mut response = Json(profile).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{COOKIE}={token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age={SESSION_SECONDS}{}",
            if s.terminal_tls { "; Secure" } else { "" }
        ))
        .map_err(|_| mismatch())?,
    );
    Ok(response)
}
async fn profile(State(s): State<ApiState>, headers: HeaderMap) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::UserProfile(identity(&s, &headers)?))
        .await?
    {
        Reply::Profile(profile) => Ok(Json(profile).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn logout(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let _: Empty = decode(&body)?;
    let token = cookie(&headers)?;
    let current = s.auth.resolve(token)?;
    let reply = s.runtime.request(Command::EndUserSession(current)).await?;
    if !matches!(reply, Reply::Done) {
        return Err(mismatch());
    }
    s.auth.remove(token)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "rx_session=; Path=/api; HttpOnly; SameSite=Strict; Max-Age=0{}",
            if s.terminal_tls { "; Secure" } else { "" }
        ))
        .map_err(|_| mismatch())?,
    );
    Ok(response)
}
async fn overview(State(s): State<ApiState>, headers: HeaderMap) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::Overview(identity(&s, &headers)?))
        .await?
    {
        Reply::Overview(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellQuery {
    id: Name,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeRestrictionsQuery {
    cell: Name,
}
async fn runtime_restrictions(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<RuntimeRestrictionsQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::RuntimeRestrictions {
            identity: identity(&s, &headers)?,
            cell: q.cell,
        })
        .await?
    {
        Reply::RuntimeRestrictions(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
async fn cell(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CellQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::InspectCell {
            identity: identity(&s, &headers)?,
            cell: q.id,
        })
        .await?
    {
        Reply::Cell(revision, value) => {
            Ok(Json(rx_application::projection::Versioned { revision, value }).into_response())
        }
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunQuery {
    id: Id,
}
async fn operator_start_context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(input): Query<rx_application::operator_start::ContextRequest>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetOperatorStartContext {
            identity: identity(&s, &headers)?,
            input,
        })
        .await?
    {
        Reply::OperatorStartContext(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
async fn operator_start_attempt(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(input): Query<rx_application::operator_start::AttemptRequest>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetOperatorStartAttempt {
            identity: identity(&s, &headers)?,
            input,
        })
        .await?
    {
        Reply::OperatorStartAttempt(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
async fn run_checkpoint(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<RunQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::RunCheckpoint {
            identity: identity(&s, &headers)?,
            run: q.id,
        })
        .await?
    {
        Reply::RunCheckpoint(snapshot) => {
            let view = rx_protocol_adapter::workflow::run_view(&snapshot);
            let value = rx_protocol::json::to_value(&view).map_err(|_| mismatch())?;
            Ok(Json(value).into_response())
        }
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactQuery {
    run: Id,
    sha256: Digest,
    schema_id: Name,
    size_bytes: Counter,
}
async fn checkpoint_artifact(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ArtifactQuery>,
) -> Result<Response, ApiError> {
    let reference = ArtifactRef {
        sha256: q.sha256,
        schema_id: q.schema_id,
        size_bytes: q.size_bytes,
    };
    match s
        .runtime
        .request(Command::CheckpointArtifact {
            identity: identity(&s, &headers)?,
            run: q.run,
            reference,
        })
        .await?
    {
        // These are the exact canonical bytes addressed by the checkpoint, not a rewrapped view.
        Reply::ArtifactBytes(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
        }
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mutation<T> {
    request_key: Id,
    command: T,
}
#[derive(Serialize)]
struct Revision {
    revision: Counter,
}
async fn install_cell(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<CellConfiguration> = decode(&body)?;
    match s
        .runtime
        .request(Command::InstallCell {
            identity: current,
            key: value.request_key,
            configuration: Box::new(value.command),
        })
        .await?
    {
        Reply::Revision(revision) => Ok(Json(Revision { revision }).into_response()),
        _ => Err(mismatch()),
    }
}
async fn create_run(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<CreateRun> = decode(&body)?;
    match s
        .runtime
        .request(Command::CreateRun {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::Run(run) => Ok(Json(run).into_response()),
        _ => Err(mismatch()),
    }
}
async fn start_run(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<StartRun> = decode(&body)?;
    match s
        .runtime
        .request(Command::StartRun {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::Attempt(attempt) => Ok(Json(attempt).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hold {
    cell: Name,
}
async fn hold(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<Hold> = decode(&body)?;
    match s
        .runtime
        .request(Command::Hold {
            identity: current,
            key: value.request_key,
            cell: value.command.cell,
        })
        .await?
    {
        Reply::Held(cell) => Ok(Json(cell).into_response()),
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseQuery {
    cell: Name,
    case: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CasesQuery {
    cell: Name,
}
async fn cases(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CasesQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::Cases {
            identity: identity(&s, &headers)?,
            cell: q.cell,
        })
        .await?
    {
        Reply::Cases(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn case_detail(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CaseQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::InspectCase {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            case: q.case,
        })
        .await?
    {
        Reply::CaseDetail(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn open_case(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::intervention::OpenCase> = decode(&body)?;
    match s
        .runtime
        .request(Command::OpenCase {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::CaseSnapshot(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn acknowledge_case(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::intervention::AcknowledgeCase> = decode(&body)?;
    match s
        .runtime
        .request(Command::AcknowledgeCase {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::CaseDetail(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

async fn record_procedure(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::procedure::Submission> = decode(&body)?;
    let Reply::ProcedureReceipt(receipt) = s
        .runtime
        .request(Command::RecordProcedure {
            identity: current,
            key: value.request_key,
            command: Box::new(value.command),
        })
        .await?
    else {
        return Err(mismatch());
    };
    if let Some(error) = receipt.transition_error {
        use rx_domain::fault::Rejection;
        let (status, code) = match error {
            Rejection::StaleRevision => (StatusCode::CONFLICT, "STALE_REVISION"),
            Rejection::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            Rejection::InvalidInput => (StatusCode::BAD_REQUEST, "INVALID_INPUT"),
            Rejection::UnsupportedSchema => {
                (StatusCode::UNPROCESSABLE_ENTITY, "UNSUPPORTED_SCHEMA")
            }
            _ => (StatusCode::UNPROCESSABLE_ENTITY, "CONDITION_UNKNOWN"),
        };
        return Ok((status,Json(serde_json::json!({"code":code,"outcome_unknown":false,"facts_recorded":receipt.facts_recorded,"case_id":receipt.case.case.id,"record_id":receipt.record.record.id,"receipt":receipt}))).into_response());
    }
    Ok(Json(receipt).into_response())
}

async fn prepare_close(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::closure::PrepareClose> = decode(&body)?;
    match s
        .runtime
        .request(Command::PrepareClose {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::Clearance(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn close_without_restart(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::closure::CloseWithoutRestart> = decode(&body)?;
    match s
        .runtime
        .request(Command::CloseWithoutRestart {
            identity: current,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::CloseReceipt(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

async fn cell_context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(cell_id): Path<String>,
) -> Result<Response, ApiError> {
    let cell = Name::new(cell_id).map_err(|_| ApiError::invalid())?;
    let Reply::Cell(revision, value) = s
        .runtime
        .request(Command::InspectCell {
            identity: identity(&s, &headers)?,
            cell,
        })
        .await?
    else {
        return Err(mismatch());
    };
    let wire = rx_protocol_adapter::cell_context::view(revision, &value).map_err(|status| {
        if status.code() == tonic::Code::FailedPrecondition {
            ApiError::new(StatusCode::CONFLICT, "UPGRADE_REQUIRED")
        } else {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "STORE_FAULT")
        }
    })?;
    let json = rx_protocol::json::to_value(&wire)
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "STORE_FAULT"))?;
    Ok(Json(json).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessDraftListQuery {
    cell: Name,
    after: Option<Id>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessDraftQuery {
    cell: Name,
    id: Id,
    revision: Option<Counter>,
}
async fn process_drafts(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListProcessDrafts {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            after: q.after,
        })
        .await?
    {
        Reply::ProcessDrafts(page) => Ok(Json(page).into_response()),
        _ => Err(mismatch()),
    }
}
async fn process_draft(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetProcessDraft {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::ProcessDraft(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
async fn save_process_draft(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let input: Mutation<rx_application::process_draft::Save> = decode(&body)?;
    let prepared = tokio::task::spawn_blocking(move || {
        rx_application::process_draft::PreparedSave::prepare(input.command)
    })
    .await
    .map_err(|_| mismatch())?
    .map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::SaveProcessDraft {
            identity: current,
            key: input.request_key,
            prepared,
        })
        .await?
    {
        Reply::ProcessDraft(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}

async fn draft_device_binding_catalog(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let input: rx_application::draft_bindings::SelectCatalog = decode(&body)?;
    match s
        .runtime
        .request(Command::DraftDeviceBindingCatalog {
            identity: identity(&s, &headers)?,
            input,
        })
        .await?
    {
        Reply::DraftBindingCatalog(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn draft_binding_catalog(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftListQuery>,
) -> Result<Response, ApiError> {
    if q.after.is_some() {
        return Err(ApiError::invalid());
    }
    match s
        .runtime
        .request(Command::DraftBindingCatalog {
            identity: identity(&s, &headers)?,
            cell: q.cell,
        })
        .await?
    {
        Reply::DraftBindingCatalog(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn draft_bindings(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetDraftBindings {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::DraftBindingView(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn save_draft_bindings(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let input: Mutation<rx_application::draft_bindings::Save> = decode(&body)?;
    match s
        .runtime
        .request(Command::SaveDraftBindings {
            identity: current,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::DraftBindingVersion(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftCompileQuery {
    cell: Name,
    id: Id,
    source_revision: Counter,
    binding_revision: Counter,
}
async fn draft_compile_input(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<DraftCompileQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ExportDraftCompileInput {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
            source_revision: q.source_revision,
            binding_revision: q.binding_revision,
        })
        .await?
    {
        Reply::DraftCompileInput(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntakeQuery {
    cell: Name,
    id: Id,
}
async fn package_intake_context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftListQuery>,
) -> Result<Response, ApiError> {
    if q.after.is_some() {
        return Err(ApiError::invalid());
    }
    match s
        .runtime
        .request(Command::PackageIntakeContext {
            identity: identity(&s, &headers)?,
            cell: q.cell,
        })
        .await?
    {
        Reply::PackageIntakeContext(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn package_intakes(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListPackageIntakes {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            after: q.after,
        })
        .await?
    {
        Reply::PackageIntakes(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_package_intake(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetPackageIntake {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::PackageIntakeView(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_package_device_catalog(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetPackageDeviceCatalog {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::PackageDeviceCatalog(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn submit_package_intake(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let current = identity(&s, &headers)?;
    let value: Mutation<rx_application::package_intake::Submit> = decode(&body)?;
    let ticket = match s
        .runtime
        .request(Command::PreparePackageIntake {
            identity: current,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::PackageIntakePreflight(rx_application::package_intake::Preflight::Recorded(v)) => {
            return Ok(Json(v).into_response());
        }
        Reply::PackageIntakePreflight(rx_application::package_intake::Preflight::Verify(
            ticket,
        )) => ticket,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PACKAGE_INTAKE_NOT_CONFIGURED",
        )
    })?;
    let prepared = worker
        .prepare(*ticket)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "PACKAGE_VERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitPackageIntake(Box::new(prepared)))
        .await?
    {
        Reply::PackageIntakeReceipt(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

async fn create_process_review(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_review::Create> = decode(&body)?;
    match s
        .runtime
        .request(Command::CreateProcessReview {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ProcessReviewJob(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_process_review(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReviewQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetProcessReviewRevision {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::ProcessReviewDetail(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn submit_review_report(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_review::Submit> = decode(&body)?;
    let ticket = match s
        .runtime
        .request(Command::PrepareReviewReport {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ReviewPreflight(rx_application::process_review::Preflight::Recorded(v)) => {
            return Ok(Json(v).into_response());
        }
        Reply::ReviewPreflight(rx_application::process_review::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "REVIEW_WORKER_UNAVAILABLE")
    })?;
    let prepared = worker
        .prepare_review(*ticket)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "VERIFICATION_REPORT_REJECTED"))?;
    match s
        .runtime
        .request(Command::CommitReviewReport(Box::new(prepared)))
        .await?
    {
        Reply::ProcessReviewVersion(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn decide_process_review(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_review::Decide> = decode(&body)?;
    let ticket = match s
        .runtime
        .request(Command::PrepareReviewDecision {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ReviewDecisionPreflight(
            rx_application::process_review::DecisionPreflight::Recorded(v),
        ) => return Ok(Json(v).into_response()),
        Reply::ReviewDecisionPreflight(
            rx_application::process_review::DecisionPreflight::Verify(t),
        ) => t,
        _ => return Err(mismatch()),
    };
    let prepared = if ticket.requires_verification() {
        let worker = s.package_intake.as_ref().ok_or_else(|| {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "REVIEW_WORKER_UNAVAILABLE")
        })?;
        worker
            .prepare_review_decision(*ticket)
            .await
            .map_err(|_| ApiError::new(StatusCode::CONFLICT, "REVIEW_REVERIFICATION_FAILED"))?
    } else {
        rx_application::process_review::PreparedDecision::reject(*ticket)
            .map_err(|_| ApiError::invalid())?
    };
    match s
        .runtime
        .request(Command::CommitReviewDecision(Box::new(prepared)))
        .await?
    {
        Reply::ProcessReviewDecision(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewQuery {
    cell: Name,
    id: Id,
    revision: Option<Counter>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewListQuery {
    cell: Name,
    intake: Id,
    after: Option<Id>,
}
async fn list_process_reviews(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReviewListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListProcessReviews {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            intake: q.intake,
            after: q.after,
        })
        .await?
    {
        Reply::ProcessReviews(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

async fn propose_process_change(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_change::Create> = decode(&body)?;
    let result = s
        .runtime
        .request(Command::PrepareProcessChange {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?;
    finish_change_worker(&s, result, ChangeCommit::Propose).await
}
async fn stage_process_change(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_change::Transition> = decode(&body)?;
    let result = s
        .runtime
        .request(Command::PrepareProcessChangeStage {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?;
    finish_change_worker(&s, result, ChangeCommit::Stage).await
}
enum ChangeCommit {
    Propose,
    Stage,
    Apply,
}
async fn apply_process_change(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_change::Transition> = decode(&body)?;
    let result = s
        .runtime
        .request(Command::PrepareProcessChangeApply {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?;
    finish_change_worker(&s, result, ChangeCommit::Apply).await
}
async fn finish_change_worker(
    s: &ApiState,
    result: Reply,
    action: ChangeCommit,
) -> Result<Response, ApiError> {
    let ticket = match result {
        Reply::ProcessChangePreflight(rx_application::process_change::Preflight::Recorded(c)) => {
            return Ok(Json(c).into_response());
        }
        Reply::ProcessChangePreflight(rx_application::process_change::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "CHANGE_WORKER_UNAVAILABLE")
    })?;
    let p = worker
        .prepare_process_change(*ticket)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "CHANGE_VERIFICATION_FAILED"))?;
    let result = s
        .runtime
        .request(match action {
            ChangeCommit::Propose => Command::CommitProcessChange(Box::new(p)),
            ChangeCommit::Stage => Command::CommitProcessChangeStage(Box::new(p)),
            ChangeCommit::Apply => Command::CommitProcessChangeApply(Box::new(p)),
        })
        .await?;
    match result {
        Reply::ProcessChange(c) => Ok(Json(c).into_response()),
        _ => Err(mismatch()),
    }
}
async fn review_change_impact(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::process_change::ReviewImpact> = decode(&body)?;
    match s
        .runtime
        .request(Command::ReviewProcessChangeImpact {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::ProcessChange(c) => Ok(Json(c).into_response()),
        _ => Err(mismatch()),
    }
}
async fn begin_change_preparation(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::process_change::BeginPreparation> = decode(&body)?;
    match s
        .runtime
        .request(Command::BeginProcessChangePreparation {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::ProcessChange(c) => Ok(Json(c).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_process_change(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetProcessChange {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::ProcessChangeDetail(c) => Ok(Json(c).into_response()),
        _ => Err(mismatch()),
    }
}

async fn authorize_host_configuration(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let value: Mutation<rx_application::process_change::Transition> = decode(&body)?;
    match s
        .runtime
        .request(Command::AuthorizeHostConfiguration {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::HostConfigurationBatch(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

async fn begin_requalification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::requalification::Begin> = decode(&body)?;
    match s
        .runtime
        .request(Command::BeginRequalification {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::RequalificationJob(j) => Ok(Json(j).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_requalification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetRequalification {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::RequalificationDetail(d) => Ok(Json(d).into_response()),
        _ => Err(mismatch()),
    }
}
async fn submit_requalification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::requalification::Submit> = decode(&body)?;
    let t = match s
        .runtime
        .request(Command::PrepareRequalificationReport {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::RequalificationPreflight(rx_application::requalification::Preflight::Recorded(
            v,
        )) => return Ok(Json(v).into_response()),
        Reply::RequalificationPreflight(rx_application::requalification::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s
        .package_intake
        .as_ref()
        .and_then(|w| w.requalification())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "REQUALIFICATION_WORKER_UNAVAILABLE",
            )
        })?;
    let p = worker
        .prepare(*t)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "REQUALIFICATION_VERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitRequalificationReport(Box::new(p)))
        .await?
    {
        Reply::RequalificationVersion(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
async fn decide_requalification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::requalification::Decide> = decode(&body)?;
    let t = match s
        .runtime
        .request(Command::PrepareRequalificationDecision {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::RequalificationDecisionPreflight(
            rx_application::requalification::DecisionPreflight::Recorded(d),
        ) => return Ok(Json(d).into_response()),
        Reply::RequalificationDecisionPreflight(
            rx_application::requalification::DecisionPreflight::Verify(t),
        ) => t,
        _ => return Err(mismatch()),
    };
    let worker = s
        .package_intake
        .as_ref()
        .and_then(|w| w.requalification())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "REQUALIFICATION_WORKER_UNAVAILABLE",
            )
        })?;
    let p = worker
        .decide(*t)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "REQUALIFICATION_REVIEW_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitRequalificationDecision(Box::new(p)))
        .await?
    {
        Reply::RequalificationDecision(d) => Ok(Json(d).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationArtifactQuery {
    review: Id,
    cell: Name,
    reference: ArtifactRef,
}
async fn requalification_artifact(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let q: QualificationArtifactQuery = decode(&body)?;
    match s
        .runtime
        .request(Command::GetRequalificationArtifact {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.review,
            reference: q.reference,
        })
        .await?
    {
        Reply::ArtifactBytes(b) => Ok((
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            b,
        )
            .into_response()),
        _ => Err(mismatch()),
    }
}

async fn issue_qualification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::qualification_activation::IssueRequest> = decode(&body)?;
    let reply = s
        .runtime
        .request(Command::PrepareQualificationIssue {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?;
    finish_qualification_worker(&s, reply, false).await
}
async fn activate_qualification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<rx_application::qualification_activation::Finalize> = decode(&body)?;
    let reply = s
        .runtime
        .request(Command::PrepareQualificationActivation {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?;
    finish_qualification_worker(&s, reply, true).await
}
async fn finish_qualification_worker(
    s: &ApiState,
    reply: Reply,
    activate: bool,
) -> Result<Response, ApiError> {
    let t = match reply {
        Reply::QualificationPreflight(
            rx_application::qualification_activation::Preflight::Recorded(b),
        ) => return Ok(Json(b).into_response()),
        Reply::QualificationPreflight(
            rx_application::qualification_activation::Preflight::Verify(t),
        ) => t,
        _ => return Err(mismatch()),
    };
    let w = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "QUALIFICATION_WORKER_UNAVAILABLE",
        )
    })?;
    let p = w
        .prepare_qualification_activation(*t)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "QUALIFICATION_VERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(if activate {
            Command::CommitQualificationActivation(Box::new(p))
        } else {
            Command::CommitQualificationIssue(Box::new(p))
        })
        .await?
    {
        Reply::QualificationBatch(b) => Ok(Json(b).into_response()),
        _ => Err(mismatch()),
    }
}
async fn get_qualification_batch(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetQualificationBatch {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::QualificationView(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuspendQualification {
    batch: Id,
    reason: Name,
}
async fn suspend_qualification(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<SuspendQualification> = decode(&body)?;
    match s
        .runtime
        .request(Command::SuspendQualification {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            batch: v.command.batch,
            reason: v.command.reason,
        })
        .await?
    {
        Reply::QualificationBatch(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
