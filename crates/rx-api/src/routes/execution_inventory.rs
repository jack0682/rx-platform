use super::*;
use rx_application::execution_inventory as data;
use rx_domain::definition::Reference;
pub(super) async fn initialize(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let input: Mutation<data::Initialize> = decode(&body)?;
    match s
        .runtime
        .request(Command::InitializeExecutionSlots {
            identity: identity(&s, &headers)?,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ExecutionSlotPool(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn pool(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(resource): Query<Reference>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetExecutionSlotPool {
            identity: identity(&s, &headers)?,
            resource,
        })
        .await?
    {
        Reply::ExecutionSlotPool(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn create_run(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let input: Mutation<data::CreateRun> = decode(&body)?;
    match s
        .runtime
        .request(Command::CreateExecutionRun {
            identity: identity(&s, &headers)?,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ExecutionRunBinding(value) => Ok(Json(
            serde_json::json!({"binding":value,"state":"PREPARED","operation_authorized":false}),
        )
        .into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunQuery {
    run: Id,
}
pub(super) async fn run(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<RunQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetExecutionRun {
            identity: identity(&s, &headers)?,
            run: q.run,
        })
        .await?
    {
        Reply::ExecutionRunBinding(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn bind_object(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let input: Mutation<data::BindObject> = decode(&body)?;
    match s
        .runtime
        .request(Command::BindExecutionObject {
            identity: identity(&s, &headers)?,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::ExecutionObjectBinding(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObjectQuery {
    run: Id,
    ordinal: Counter,
}
pub(super) async fn object(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ObjectQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetExecutionObject {
            identity: identity(&s, &headers)?,
            run: q.run,
            ordinal: q.ordinal,
        })
        .await?
    {
        Reply::ExecutionObjectBinding(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
