//! Terminal-authenticated recovery communication. The worker/writer owns every business decision.
use super::*;
use axum::extract::rejection::QueryRejection;
use rx_application::{Role, host_recovery as recovery};
use rx_runtime::host_recovery::{Service, WorkerError};
use std::collections::BTreeMap;

#[derive(Serialize)]
struct ContextResponse {
    context: recovery::Context,
    context_digest: Digest,
    expected_cells: BTreeMap<Name, Counter>,
}
#[derive(Serialize)]
struct ViewResponse {
    view: recovery::View,
    proposal_digest: Digest,
}
#[derive(Serialize)]
struct PageResponse {
    items: Vec<ViewResponse>,
    next: Option<Id>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextQuery {
    host: Name,
    origin: Name,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    host: Name,
    after: Option<Id>,
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingId {
    id: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperationQuery {
    id: Id,
    operation: Id,
}

/// Refresh role/session/terminal before revealing worker availability. The worker repeats
/// exact cell/cohort authorization in its authoritative read/transaction before transport I/O.
async fn release_identity(s: &ApiState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let identity = identity(s, headers)?;
    let Reply::Profile(profile) = s
        .runtime
        .request(Command::UserProfile(identity.clone()))
        .await?
    else {
        return Err(mismatch());
    };
    if !s.terminal_tls
        || identity.terminal.is_none()
        || profile.terminal.is_none()
        || !profile.roles.contains(&Role::ReleaseManager)
    {
        return Err(ApiError::forbidden());
    }
    Ok(identity)
}
fn service(s: &ApiState) -> Result<&Arc<dyn Service>, ApiError> {
    s.host_recovery.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "HOST_RECOVERY_NOT_CONFIGURED",
        )
    })
}
fn worker_error(error: WorkerError) -> ApiError {
    match error {
        WorkerError::Writer(error) => error.into(),
        WorkerError::Unavailable => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "HOST_RECOVERY_UNAVAILABLE",
            outcome_unknown: true,
        },
        WorkerError::InvalidRead => {
            ApiError::new(StatusCode::CONFLICT, "HOST_RECOVERY_INVALID_READ")
        }
        WorkerError::Busy => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "HOST_RECOVERY_BUSY"),
    }
}
fn view_response(value: recovery::View, expected: Option<&Id>) -> Result<ViewResponse, ApiError> {
    // A miswired service cannot turn this recovery-only response into an operation permit.
    if value.operation_authorized || expected.is_some_and(|id| id != &value.binding.id) {
        return Err(mismatch());
    }
    let proposal_digest = value.binding.proposal_digest().map_err(|_| mismatch())?;
    Ok(ViewResponse {
        view: value,
        proposal_digest,
    })
}
fn view(value: recovery::View, expected: Option<&Id>) -> Result<Response, ApiError> {
    Ok(Json(view_response(value, expected)?).into_response())
}

pub(super) async fn context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<ContextQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    let result = service(&s)?
        .context(identity, input.host.clone(), input.origin.clone())
        .await
        .map_err(worker_error)?;
    if result.host != input.host || result.origin != input.origin {
        return Err(mismatch());
    }
    let context_digest = result.digest().map_err(|_| mismatch())?;
    let expected_cells = result.expected_cells();
    Ok(Json(ContextResponse {
        context: result,
        context_digest,
        expected_cells,
    })
    .into_response())
}
pub(super) async fn list(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    if !(1..=50).contains(&input.limit) {
        return Err(ApiError::invalid());
    }
    let page = service(&s)?
        .list(identity, input.host.clone(), input.after, input.limit)
        .await
        .map_err(worker_error)?;
    if page.items.len() > input.limit
        || page
            .items
            .iter()
            .any(|v| v.binding.context.host != input.host)
    {
        return Err(mismatch());
    }
    let items = page
        .items
        .into_iter()
        .map(|value| view_response(value, None))
        .collect::<Result<_, _>>()?;
    Ok(Json(PageResponse {
        items,
        next: page.next,
    })
    .into_response())
}
pub(super) async fn propose(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let input: Mutation<recovery::Prepare> = decode(&body)?;
    let expected = input.command.clone();
    let principal = identity.principal.clone();
    let result = service(&s)?
        .propose(identity, input.request_key, input.command)
        .await
        .map_err(worker_error)?;
    let binding = &result.binding;
    if binding.context.host != expected.host
        || binding.context.origin != expected.origin
        || binding.requested_context_digest != expected.expected_context
        || binding.context.expected_cells() != expected.expected_cells
        || binding.proposed_by.principal != principal
    {
        return Err(mismatch());
    }
    // A recovered receipt retains the original session/terminal. Current access was checked
    // above and must also be checked by the worker; historical attribution is not authentication.
    view(result, None)
}
pub(super) async fn approve(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let input: Mutation<recovery::Approve> = decode(&body)?;
    let id = input.command.id.clone();
    view(
        service(&s)?
            .approve(identity, input.request_key, input.command)
            .await
            .map_err(worker_error)?,
        Some(&id),
    )
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<BindingId>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    view(
        service(&s)?
            .get(identity, input.id.clone())
            .await
            .map_err(worker_error)?,
        Some(&input.id),
    )
}
pub(super) async fn progress(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let input: BindingId = decode(&body)?;
    view(
        service(&s)?
            .progress(identity, input.id.clone())
            .await
            .map_err(worker_error)?,
        Some(&input.id),
    )
}
pub(super) async fn query(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = release_identity(&s, &headers).await?;
    let input: OperationQuery = decode(&body)?;
    let result = service(&s)?
        .query(identity, input.id.clone(), input.operation.clone())
        .await
        .map_err(worker_error)?;
    if result.operation_authorized
        || result.evidence_complete
        || result.binding != input.id
        || result.operation != input.operation
    {
        return Err(mismatch());
    }
    Ok(Json(result).into_response())
}
