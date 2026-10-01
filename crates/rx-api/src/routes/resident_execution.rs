use super::*;
use rx_domain::resident_execution as execution;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryId {
    id: Id,
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<QueryId>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(q) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::GetResidentExecution { identity, id: q.id })
        .await?
    {
        Reply::ResidentExecution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn propose(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<execution::Propose> = decode(&body)?;
    match s
        .runtime
        .request(Command::ProposeResidentExecution {
            identity,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ResidentExecution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn approve(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<execution::Approve> = decode(&body)?;
    match s
        .runtime
        .request(Command::ApproveResidentExecution {
            identity,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ResidentExecution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn stop(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<execution::Stop> = decode(&body)?;
    match s
        .runtime
        .request(Command::StopResidentExecution {
            identity,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ResidentExecution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
