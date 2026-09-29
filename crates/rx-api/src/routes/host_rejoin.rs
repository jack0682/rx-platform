//! Current local prerequisites only; no transport I/O or approval occurs here.
use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryContext {
    host: Name,
    origin: Name,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProposalQuery {
    id: Id,
}
pub(super) async fn propose(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: Mutation<rx_application::host_rejoin::Prepare> = decode(&body)?;
    let expected = input.command.clone();
    let result = host_recovery::service(&s)?
        .propose_rejoin(identity, input.request_key, input.command)
        .await
        .map_err(host_recovery::worker_error)?;
    if result.operation_authorized
        || result.proposal.context.host != expected.host
        || result.proposal.context.origin != expected.origin
        || result.proposal.context_digest != expected.expected_context
        || result.proposal.context.expected_cells() != expected.expected_cells
    {
        return Err(mismatch());
    }
    Ok(Json(result).into_response())
}
pub(super) async fn proposal(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: std::result::Result<Query<ProposalQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let Query(q) = query.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY"))?;
    match s
        .runtime
        .request(Command::GetHostRejoinProposal {
            identity,
            id: q.id.clone(),
        })
        .await?
    {
        Reply::HostRejoinProposal(v) if !v.operation_authorized && v.proposal.id == q.id => {
            Ok(Json(*v).into_response())
        }
        _ => Err(mismatch()),
    }
}
#[derive(Serialize)]
struct ContextResponse {
    context: rx_application::host_rejoin::Context,
    context_digest: Digest,
}
pub(super) async fn context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: std::result::Result<Query<QueryContext>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    if !s.terminal_tls {
        return Err(ApiError::forbidden());
    }
    let Query(q) = query.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY"))?;
    match s
        .runtime
        .request(Command::HostRejoinContext {
            identity: identity(&s, &headers)?,
            host: q.host,
            origin: q.origin,
        })
        .await?
    {
        Reply::HostRejoinContext(c) => {
            let context_digest = c.digest().map_err(|_| mismatch())?;
            Ok(Json(ContextResponse {
                context: *c,
                context_digest,
            })
            .into_response())
        }
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingInput {
    id: Id,
}
pub(super) async fn approve(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: Mutation<rx_application::host_rejoin::Approve> = decode(&body)?;
    let id = input.command.id.clone();
    let value = host_recovery::service(&s)?
        .approve_rejoin(identity, input.request_key, input.command)
        .await
        .map_err(host_recovery::worker_error)?;
    if value.operation_authorized || value.binding.id != id {
        return Err(mismatch());
    }
    Ok(Json(value).into_response())
}
pub(super) async fn progress(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: BindingInput = decode(&body)?;
    let value = host_recovery::service(&s)?
        .progress_rejoin(identity, input.id.clone())
        .await
        .map_err(host_recovery::worker_error)?;
    if value.operation_authorized || value.binding.id != input.id {
        return Err(mismatch());
    }
    Ok(Json(value).into_response())
}
pub(super) async fn binding(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: std::result::Result<Query<ProposalQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let Query(q) = query.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY"))?;
    match s
        .runtime
        .request(Command::GetHostRejoinBinding {
            identity,
            id: q.id.clone(),
        })
        .await?
    {
        Reply::HostRejoinBindingView(v) if !v.operation_authorized && v.binding.id == q.id => {
            Ok(Json(*v).into_response())
        }
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationInput {
    id: Id,
    operation: Id,
}
pub(super) async fn query(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: OperationInput = decode(&body)?;
    let value = host_recovery::service(&s)?
        .query_rejoin(identity, input.id.clone(), input.operation.clone())
        .await
        .map_err(host_recovery::worker_error)?;
    if value.result.operation_authorized
        || value.binding != input.id
        || value.operation != input.operation
    {
        return Err(mismatch());
    }
    Ok(Json(value).into_response())
}

pub(super) async fn settle(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: Mutation<rx_application::host_rejoin::ApproveSettlement> = decode(&body)?;
    let operation = input.command.settlement.operation.clone();
    let binding = input.command.reference.binding.clone();
    let result = host_recovery::service(&s)?
        .settle_rejoin(identity, input.request_key, input.command)
        .await
        .map_err(host_recovery::worker_error)?;
    if result.operation != operation || result.rejoin.as_ref().is_none_or(|r| r.binding != binding)
    {
        return Err(mismatch());
    }
    Ok(Json(result).into_response())
}

pub(super) async fn rebind(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: Mutation<rx_application::host_rejoin::ApproveRebind> = decode(&body)?;
    let binding = input.command.binding.clone();
    let value = host_recovery::service(&s)?
        .rebind(identity, input.request_key, input.command)
        .await
        .map_err(host_recovery::worker_error)?;
    if value.production_authorized || value.rebind.binding != binding {
        return Err(mismatch());
    }
    Ok(Json(value).into_response())
}
pub(super) async fn rebind_progress(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let input: BindingInput = decode(&body)?;
    let value = host_recovery::service(&s)?
        .progress_rebind(identity, input.id.clone())
        .await
        .map_err(host_recovery::worker_error)?;
    if value.production_authorized || value.rebind.id != input.id {
        return Err(mismatch());
    }
    Ok(Json(value).into_response())
}
pub(super) async fn rebind_get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: std::result::Result<Query<ProposalQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    let Query(q) = query.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "INVALID_QUERY"))?;
    match s
        .runtime
        .request(Command::GetHostRebind {
            identity,
            id: q.id.clone(),
        })
        .await?
    {
        Reply::HostRebindView(v) if !v.production_authorized && v.rebind.id == q.id => {
            Ok(Json(*v).into_response())
        }
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RestrictionQuery {
    cell: Name,
}
pub(super) async fn restrictions(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<RestrictionQuery>,
) -> Result<Response, ApiError> {
    let identity = host_recovery::release_identity(&s, &headers).await?;
    match s
        .runtime
        .request(Command::HostRebindRestrictions {
            identity,
            cell: q.cell.clone(),
        })
        .await?
    {
        Reply::HostRebindRestrictions(v) if v.cell == q.cell && !v.clearance_authorized => {
            Ok(Json(*v).into_response())
        }
        _ => Err(mismatch()),
    }
}
