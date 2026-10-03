use super::*;
use rx_application::workflow_publication as publication;
use rx_domain::definition::Reference;

fn input_error(message: String) -> Response {
    let stale = message.starts_with("STALE_EXECUTION_REFERENCE");
    (
        if stale {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        },
        Json(serde_json::json!({
            "code":if stale {"STALE_EXECUTION_REFERENCE"}else{"INVALID_EXECUTION_INPUT"},
            "message":message,"outcome_unknown":false,
        })),
    )
        .into_response()
}
pub(super) async fn configuration(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let configuration: CellConfiguration = decode(&body)?;
    match command(
        &s,
        Command::PrepareWorkflowConfiguration {
            identity: identity(&s, &headers)?,
            configuration: Box::new(configuration),
        },
    )
    .await
    {
        Ok(Reply::WorkflowConfiguration(reference)) => Ok(Json(
            serde_json::json!({"configuration":reference,"installed":false,"qualified":false}),
        )
        .into_response()),
        Err(response) => Ok(*response),
        _ => Err(mismatch()),
    }
}
async fn command(s: &ApiState, input: Command) -> Result<Reply, Box<Response>> {
    match s.runtime.request(input).await {
        Ok(reply) => Ok(reply),
        Err(rx_runtime::writer::WriterError::Rejected(rx_ports::StoreError::Invalid(message))) => {
            Err(Box::new(input_error(message)))
        }
        Err(error) => Err(Box::new(ApiError::from(error).into_response())),
    }
}
pub(super) async fn preview(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<publication::PreviewInput> = decode(&body)?;
    let result = command(
        &s,
        Command::PrepareExecutionPreview {
            identity: identity.clone(),
            key: input.request_key.clone(),
            input: input.command,
        },
    )
    .await;
    let work = match result {
        Ok(Reply::ExecutionPreviewPreparation(preparation)) => match *preparation {
            publication::Preparation::Recorded(record) => return Ok(Json(record).into_response()),
            publication::Preparation::Pending(work) => work,
        },
        Err(response) => return Ok(*response),
        _ => return Err(mismatch()),
    };
    let prepared =
        match tokio::task::spawn_blocking(move || publication::PreparedPreview::prepare(*work))
            .await
            .map_err(|_| ApiError::unavailable())?
        {
            Ok(value) => value,
            Err(error) => return Ok(input_error(error)),
        };
    match command(
        &s,
        Command::SaveExecutionPreview {
            identity,
            key: input.request_key,
            prepared: Box::new(prepared),
        },
    )
    .await
    {
        Ok(Reply::ExecutionPreview(value)) => Ok(Json(value).into_response()),
        Err(response) => Ok(*response),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get_preview(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(reference): Query<Reference>,
) -> Result<Response, ApiError> {
    match command(&s,Command::GetExecutionPreview {identity:identity(&s,&headers)?,reference}).await {
        Ok(Reply::SavedExecutionPreview(value))=>Ok(Json(serde_json::json!({"preview":value.preview(),"policy":value.policy(),"qualification":"NOT_QUALIFIED"})).into_response()),
        Err(response)=>Ok(*response),_=>Err(mismatch()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportQuery {
    catalog: Id,
    id: Id,
    revision: Counter,
    digest: Digest,
    candidate: u8,
    slot: u16,
}
pub(super) async fn report(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReportQuery>,
) -> Result<Response, ApiError> {
    let reference = Reference {
        catalog: q.catalog,
        id: q.id,
        revision: q.revision,
        digest: q.digest,
    };
    let saved = match command(
        &s,
        Command::GetExecutionPreview {
            identity: identity(&s, &headers)?,
            reference,
        },
    )
    .await
    {
        Ok(Reply::SavedExecutionPreview(value)) => value,
        Err(response) => return Ok(*response),
        _ => return Err(mismatch()),
    };
    match tokio::task::spawn_blocking(move || {
        saved
            .report(q.candidate, q.slot)
            .map(|r| r.report().to_vec())
    })
    .await
    .map_err(|_| ApiError::unavailable())?
    {
        // Preserve the approved artifact's canonical bytes for receipt hash comparison.
        Ok(bytes) => Ok((
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response()),
        Err(error) => Ok(input_error(error)),
    }
}
pub(super) async fn publish(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let input: Mutation<publication::Publish> = decode(&body)?;
    let ticket = match command(
        &s,
        Command::PublishWorkflowExecution {
            identity: identity(&s, &headers)?,
            key: input.request_key,
            input: input.command,
        },
    )
    .await
    {
        Ok(Reply::WorkflowPublicationPreparation(value)) => match *value {
            publication::PublishPreparation::Recorded(value) => {
                return Ok(Json(value).into_response());
            }
            publication::PublishPreparation::Verify(ticket) => ticket,
        },
        Err(response) => return Ok(*response),
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PACKAGE_INTAKE_NOT_CONFIGURED",
        )
    })?;
    let prepared = worker
        .prepare_workflow_publication(*ticket)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::CONFLICT,
                "EXECUTION_TEMPLATE_REVERIFICATION_FAILED",
            )
        })?;
    match command(&s, Command::CommitWorkflowPublication(Box::new(prepared))).await {
        Ok(Reply::WorkflowPublication(value)) => Ok(Json(value).into_response()),
        Err(response) => Ok(*response),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get_publication(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(reference): Query<Reference>,
) -> Result<Response, ApiError> {
    match command(
        &s,
        Command::GetWorkflowPublication {
            identity: identity(&s, &headers)?,
            reference,
        },
    )
    .await
    {
        Ok(Reply::WorkflowPublication(value)) => Ok(Json(value).into_response()),
        Err(response) => Ok(*response),
        _ => Err(mismatch()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialQuery {
    catalog: Id,
    id: Id,
    revision: Counter,
    digest: Digest,
    artifact: String,
}
/// Authenticated byte-preserving export for the installation-owned Host material store.
pub(super) async fn material(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<MaterialQuery>,
) -> Result<Response, ApiError> {
    let reference = Reference {
        catalog: q.catalog,
        id: q.id,
        revision: q.revision,
        digest: q.digest,
    };
    match command(
        &s,
        Command::GetExecutionPreview {
            identity: identity(&s, &headers)?,
            reference,
        },
    )
    .await
    {
        Ok(Reply::SavedExecutionPreview(value)) => match value.artifact(&q.artifact) {
            Ok(bytes) => Ok((
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                bytes,
            )
                .into_response()),
            Err(error) => Ok(input_error(error)),
        },
        Err(response) => Ok(*response),
        _ => Err(mismatch()),
    }
}
