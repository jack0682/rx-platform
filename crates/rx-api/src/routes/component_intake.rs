use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceQuery {
    source: Name,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportQuery {
    id: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryQuery {
    id: Id,
    #[serde(default)]
    after: Option<Counter>,
}
pub(super) async fn context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<SourceQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::ComponentIntakeContext {
            identity,
            source: query.source,
        })
        .await?
    {
        Reply::ComponentIntakeContext(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn import(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<rx_application::component_intake::Submit> = decode(&body)?;
    match s
        .runtime
        .request(Command::ImportComponents {
            identity,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ComponentIntakeReceipt(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn progress(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<ImportQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::ComponentIntakeProgress {
            identity,
            id: query.id,
        })
        .await?
    {
        Reply::ComponentIntakeProgress(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn receipt(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<ImportQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::ComponentIntakeReceipt {
            identity,
            id: query.id,
        })
        .await?
    {
        Reply::ComponentIntakeReceipt(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn history(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::ComponentIntakeHistory {
            identity,
            id: query.id,
            after: query.after.unwrap_or(Counter(0)),
        })
        .await?
    {
        Reply::ComponentIntakeHistory(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
