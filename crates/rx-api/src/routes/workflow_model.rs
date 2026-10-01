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

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ReportView {
    Details,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportList {
    catalog: Id,
    after: Option<Name>,
    view: Option<ReportView>,
}
pub(super) async fn reports(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReportList>,
) -> Result<Response, ApiError> {
    let actor = identity(&s, &headers)?;
    let Reply::WorkflowResolutions(page) = s
        .runtime
        .request(Command::ListWorkflowResolutions {
            identity: actor.clone(),
            catalog: q.catalog.clone(),
            after: q.after,
        })
        .await?
    else {
        return Err(mismatch());
    };
    if q.view.is_none() {
        return Ok(Json(page).into_response());
    }
    // Additive read projection: legacy clients retain the original strict response shape.
    // Each immutable receipt/model is read through the same catalog authorization boundary.
    let mut models = std::collections::BTreeMap::new();
    let mut reports = Vec::new();
    for summary in &page.reports {
        let Reply::WorkflowResolution(receipt) = s
            .runtime
            .request(Command::GetWorkflowResolution {
                identity: actor.clone(),
                catalog: q.catalog.clone(),
                id: summary.reference.id.clone(),
            })
            .await?
        else {
            return Err(mismatch());
        };
        if receipt.reference != summary.reference
            || receipt.report.request.workflow != summary.workflow
        {
            return Err(mismatch());
        }
        let reference = &receipt.report.request.workflow;
        if !models.contains_key(reference) {
            let Reply::WorkflowModel(value) = s
                .runtime
                .request(Command::GetWorkflowModel {
                    identity: actor.clone(),
                    catalog: reference.catalog.clone(),
                    id: reference.id.clone(),
                    revision: Some(reference.revision),
                })
                .await?
            else {
                return Err(mismatch());
            };
            if value.reference != *reference {
                return Err(mismatch());
            }
            models.insert(reference.clone(), value);
        }
        let mut contexts = models[reference].spec.defaults.clone();
        contexts.extend(receipt.report.request.contexts.clone());
        let context_refs: std::collections::BTreeSet<_> = contexts.values().flatten().collect();
        let names: Vec<_> = receipt
            .report
            .definitions
            .iter()
            .filter(|d| context_refs.contains(&d.reference))
            .collect();
        reports.push(serde_json::json!({
            "reference":receipt.reference,"workflow":reference,"slot_index":summary.slot_index,
            "status":summary.status,"created_at":receipt.created_at,"created_by":receipt.created_by,
            "contexts":contexts,"overrides":receipt.report.request.overrides,
            "definitions":names,
        }));
    }
    Ok(Json(
        serde_json::json!({"schema":"rx.workflow-resolution-index.v1", "catalog":page.catalog,
        "reports":reports,"next":page.next}),
    )
    .into_response())
}
