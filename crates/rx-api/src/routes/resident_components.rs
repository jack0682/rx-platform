//! Component authoring through the existing authenticated application writer.
use super::*;
use axum::extract::rejection::QueryRejection;
use rx_application::resident_component;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryInput {
    id: Id,
    revision: Option<Counter>,
}

pub(super) async fn get_component(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<QueryInput>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::GetComponent {
            identity,
            id: query.id,
            revision: query.revision,
        })
        .await?
    {
        Reply::Component(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn create_component(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_component::Create> = decode(&body)?;
    match s
        .runtime
        .request(Command::CreateComponent {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::Component(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn update_component(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_component::Update> = decode(&body)?;
    match s
        .runtime
        .request(Command::UpdateComponent {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::Component(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn retire_component(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_component::Retire> = decode(&body)?;
    match s
        .runtime
        .request(Command::RetireComponent {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::Component(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}
