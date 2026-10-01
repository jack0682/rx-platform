use super::*;
use rx_application::definition_catalog as catalog;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogList {
    after: Option<Name>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogQuery {
    id: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DefinitionQuery {
    catalog: Id,
    id: Id,
    revision: Option<Counter>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryQuery {
    catalog: Id,
    id: Id,
    before: Option<Counter>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    catalog: Id,
    after: Option<Name>,
    q: Option<String>,
    kind: Option<rx_domain::definition::Kind>,
    archived: Option<bool>,
}
pub(super) async fn catalogs(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CatalogList>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListDefinitionCatalogs {
            identity: identity(&s, &headers)?,
            after: q.after,
        })
        .await?
    {
        Reply::DefinitionCatalogs(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn catalog(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CatalogQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetDefinitionCatalog {
            identity: identity(&s, &headers)?,
            id: q.id,
        })
        .await?
    {
        Reply::DefinitionCatalog(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn save_catalog(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<catalog::CatalogSave> = decode(&body)?;
    match s
        .runtime
        .request(Command::SaveDefinitionCatalog {
            identity,
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::DefinitionCatalog(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<DefinitionQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetDefinition {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::Definition(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn list(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListDefinitions {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            after: q.after,
            filter: catalog::Filter {
                query: q.q.unwrap_or_default(),
                kind: q.kind,
                archived: q.archived,
            },
        })
        .await?
    {
        Reply::Definitions(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn history(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::DefinitionHistory {
            identity: identity(&s, &headers)?,
            catalog: q.catalog,
            id: q.id,
            before: q.before,
        })
        .await?
    {
        Reply::DefinitionHistory(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn save(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = identity(&s, &headers)?;
    let input: Mutation<catalog::Save> = decode(&body)?;
    let prepared = tokio::task::spawn_blocking(move || catalog::Prepared::prepare(input.command))
        .await
        .map_err(|_| ApiError::invalid())?
        .map_err(|_| ApiError::invalid())?;
    match s
        .runtime
        .request(Command::SaveDefinition {
            identity,
            key: input.request_key,
            prepared,
        })
        .await?
    {
        Reply::Definition(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}

pub(super) async fn points(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let query: rx_domain::definition::pattern::Query = decode(&body)?;
    match s
        .runtime
        .request(Command::DefinitionPoints {
            identity: identity(&s, &headers)?,
            query,
        })
        .await?
    {
        Reply::DefinitionPoints(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
