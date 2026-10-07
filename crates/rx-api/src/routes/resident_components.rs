//! Component authoring through the existing authenticated application writer.
use super::*;
use axum::extract::rejection::QueryRejection;
use rx_application::resident_component;
use rx_application::resident_reporting;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryInput {
    id: Id,
    revision: Option<Counter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportQuery {
    component: Id,
    instance: Id,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScopeQuery {
    id: Id,
}

pub(super) async fn get_reporting_scope(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<ScopeQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::ReadResidentReportingScope {
            identity,
            scope: query.id,
        })
        .await?
    {
        Reply::ResidentReportingScopeView(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn issue_reporting(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_reporting::Issue> = decode(&body)?;
    match s
        .runtime
        .request(Command::IssueResidentReporting {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ResidentReportingScopeView(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn revoke_reporting(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_reporting::Revoke> = decode(&body)?;
    match s
        .runtime
        .request(Command::RevokeResidentReporting {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ResidentReportingScopeView(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get_report(
    State(s): State<ApiState>,
    headers: HeaderMap,
    query: Result<Query<ReportQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let Query(query) = query.map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::GetResidentReport {
            identity,
            component: query.component,
            instance: query.instance,
        })
        .await?
    {
        Reply::ResidentReportView(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
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

pub(super) async fn continue_reporting(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let value: Mutation<resident_reporting::Continue> = decode(&body)?;
    match s
        .runtime
        .request(Command::ContinueResidentReporting {
            identity,
            key: value.request_key,
            input: value.command,
        })
        .await?
    {
        Reply::ResidentReportingScopeView(view) => Ok(Json(view).into_response()),
        _ => Err(mismatch()),
    }
}
