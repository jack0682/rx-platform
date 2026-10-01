use super::*;
use rx_application::workflow_model as model;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Get {
    catalog: Id,
    id: Id,
    revision: Option<Counter>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct List {
    catalog: Id,
    after: Option<Name>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    catalog: Id,
    id: Id,
}
pub(super) async fn save(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<model::Save> = decode(&body)?;
    let prepared = tokio::task::spawn_blocking(move || model::PreparedSave::prepare(input.command))
        .await
        .map_err(|_| ApiError::invalid())?
        .map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::SaveWorkflowModel {
            identity,
            key: input.request_key,
            prepared,
        })
        .await?
    {
        Reply::WorkflowModel(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<Get>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetWorkflowModel {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::WorkflowModel(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn list(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<List>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListWorkflowModels {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            after: q.after,
        })
        .await?
    {
        Reply::WorkflowModels(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn resolve(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<rx_domain::workflow::Request> = decode(&body)?;
    let Reply::WorkflowResolutionPreparation(preparation) = s
        .runtime
        .request(Command::PrepareWorkflowResolution {
            identity: identity.clone(),
            key: input.request_key.clone(),
            input: input.command,
        })
        .await?
    else {
        return Err(mismatch());
    };
    let snapshot = match *preparation {
        model::Preparation::Recorded(v) => return Ok(Json(v).into_response()),
        model::Preparation::Pending(snapshot) => *snapshot,
    };
    let prepared =
        tokio::task::spawn_blocking(move || model::PreparedResolution::prepare(snapshot))
            .await
            .map_err(|_| ApiError::invalid())?
            .map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::SaveWorkflowResolution {
            identity,
            key: input.request_key,
            prepared,
        })
        .await?
    {
        Reply::WorkflowResolution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn report(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<Report>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetWorkflowResolution {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            id: q.id,
        })
        .await?
    {
        Reply::WorkflowResolution(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn reports(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<List>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListWorkflowResolutions {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            after: q.after,
        })
        .await?
    {
        Reply::WorkflowResolutions(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
