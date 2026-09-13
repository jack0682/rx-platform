//! Human attestations and conservative disposition. No execution or resource release authority.
use super::*;
use axum::extract::rejection::QueryRejection;
use rx_application::{Role, investigation as data};
use rx_domain::operation::{Disposition, Knowledge, Outcome};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperationQuery {
    operation: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordQuery {
    operation: Id,
    id: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    operation: Id,
    after: Option<Id>,
    limit: usize,
}
#[derive(Serialize)]
struct ContextResponse {
    context: data::Context,
    context_digest: Digest,
    operation_authorized: bool,
    resource_release_authorized: bool,
}
#[derive(Serialize)]
struct AttestationResponse {
    attestation: data::Attestation,
    reference: ArtifactRef,
    current: bool,
    outcome_changed: bool,
    operation_authorized: bool,
    resource_release_authorized: bool,
}
#[derive(Serialize)]
struct DispositionResponse {
    receipt: data::Receipt,
    reference: ArtifactRef,
    current: bool,
    operation_authorized: bool,
    resource_release_authorized: bool,
}
#[derive(Serialize)]
struct AttestationPageResponse {
    items: Vec<AttestationResponse>,
    next: Option<Id>,
    operation_authorized: bool,
    resource_release_authorized: bool,
}
#[derive(Serialize)]
struct DispositionPageResponse {
    items: Vec<DispositionResponse>,
    next: Option<Id>,
    operation_authorized: bool,
    resource_release_authorized: bool,
}

/// The writer repeats current operation/cell/cohort access at preflight and commit. A cached
/// receipt still needs a currently authenticated human; its stored Actor is historical data.
async fn investigation_identity(s: &ApiState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let identity = identity(s, headers)?;
    let Reply::Profile(profile) = s
        .runtime
        .request(Command::UserProfile(identity.clone()))
        .await?
    else {
        return Err(mismatch());
    };
    if !s.terminal_tls
        || identity.terminal.is_none()
        || profile.terminal.is_none()
        || !profile.roles.contains(&Role::RecoveryLead)
        || !rx_application::procedure::can_report(&profile.roles)
    {
        return Err(ApiError::forbidden());
    }
    Ok(identity)
}

fn attestation_response(view: data::AttestationView) -> Result<AttestationResponse, ApiError> {
    let value = view.attestation;
    if view.operation_authorized
        || view.resource_release_authorized
        || value.operation_authorized
        || value.resource_release_authorized
        || value.context.operation_authorized
        || value.context.resource_release_authorized
        || value.schema.as_str() != data::ATTESTATION_SCHEMA
        || value.id != value.request.id
        || &value.request.operation != value.context.work.operation.id()
        || value.procedure_reference.sha256 != value.request.procedure_digest
    {
        return Err(mismatch());
    }
    let reference = value.reference().map_err(|_| mismatch())?;
    Ok(AttestationResponse {
        attestation: value,
        reference,
        current: view.current,
        outcome_changed: false,
        operation_authorized: false,
        resource_release_authorized: false,
    })
}
fn disposition_response(view: data::ReceiptView) -> Result<DispositionResponse, ApiError> {
    let value = view.receipt;
    if view.operation_authorized
        || view.resource_release_authorized
        || value.schema.as_str() != data::DISPOSITION_SCHEMA
        || value.operation_authorized
        || value.resource_release_authorized
        || value.resources_released
        || value.before.operation.id() != &value.request.operation
        || value.after.operation.id() != &value.request.operation
        || value.before.operation.outcome() != Outcome::None
        || value.before.operation.knowledge() != Knowledge::Unknown
        || value.before.operation.disposition() != Disposition::Quarantined
        || value.after.operation.outcome() != Outcome::Unresolved
        || value.after.operation.disposition() != Disposition::Quarantined
    {
        return Err(mismatch());
    }
    let reference = value.reference().map_err(|_| mismatch())?;
    Ok(DispositionResponse {
        receipt: value,
        reference,
        current: view.current,
        operation_authorized: false,
        resource_release_authorized: false,
    })
}

fn worker(s: &ApiState) -> Result<&Arc<rx_runtime::investigation::Worker>, ApiError> {
    s.investigation.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "INVESTIGATION_NOT_CONFIGURED",
        )
    })
}
fn verification_error(error: rx_runtime::investigation::Error) -> ApiError {
    use rx_runtime::investigation::Error;
    // Verification runs off the writer and has not committed this request's mutation.
    // Preserve only the fixed classification, never filesystem or signature error text.
    match error {
        Error::Busy => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "INVESTIGATION_BUSY"),
        Error::Unavailable => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "INVESTIGATION_UNAVAILABLE"),
        Error::VerificationFailed => ApiError::new(StatusCode::CONFLICT, "INVESTIGATION_VERIFICATION_FAILED"),
    }
}
// Match the core idempotency fingerprints. CAS is checked for a new transaction; a recovered
// record retains its historical CAS and actor session/terminal after current access is checked.
fn same_attestation_request(a: &data::AttestSubmit, b: &data::AttestSubmit) -> bool {
    a.id == b.id
        && a.operation == b.operation
        && a.context_digest == b.context_digest
        && a.procedure_digest == b.procedure_digest
        && a.assertion == b.assertion
        && a.note == b.note
        && a.occurred_at == b.occurred_at
        && a.evidence_ids.iter().collect::<BTreeSet<_>>() == b.evidence_ids.iter().collect()
}
fn same_disposition_request(a: &data::RecordDisposition, b: &data::RecordDisposition) -> bool {
    a.operation == b.operation
        && a.procedure_digest == b.procedure_digest
        && a.disposition == b.disposition
        && a.reason == b.reason
        && a.evidence_ids.iter().collect::<BTreeSet<_>>() == b.evidence_ids.iter().collect()
}
async fn read_attestation(
    s: &ApiState,
    identity: Identity,
    operation: Id,
    id: Id,
) -> Result<data::AttestationView, ApiError> {
    let Reply::InvestigationAttestationView(value) = s
        .runtime
        .request(Command::GetInvestigationAttestation {
            identity,
            operation: operation.clone(),
            id: id.clone(),
        })
        .await?
    else {
        return Err(mismatch());
    };
    if value.attestation.id != id || value.attestation.request.operation != operation {
        return Err(mismatch());
    }
    Ok(*value)
}
async fn read_disposition(
    s: &ApiState,
    identity: Identity,
    operation: Id,
    id: Id,
) -> Result<data::ReceiptView, ApiError> {
    let Reply::InvestigationReceiptView(value) = s
        .runtime
        .request(Command::GetInvestigationDisposition {
            identity,
            operation: operation.clone(),
            id: id.clone(),
        })
        .await?
    else {
        return Err(mismatch());
    };
    if value.receipt.id != id || value.receipt.request.operation != operation {
        return Err(mismatch());
    }
    Ok(*value)
}

pub(super) async fn context(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<OperationQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    let Reply::InvestigationContext(value) = s
        .runtime
        .request(Command::InvestigationContext {
            identity,
            operation: input.operation.clone(),
        })
        .await?
    else {
        return Err(mismatch());
    };
    if value.work.operation.id() != &input.operation
        || value.operation_authorized
        || value.resource_release_authorized
    {
        return Err(mismatch());
    }
    let context_digest = value.digest().map_err(|_| mismatch())?;
    Ok(Json(ContextResponse {
        context: *value,
        context_digest,
        operation_authorized: false,
        resource_release_authorized: false,
    })
    .into_response())
}
pub(super) async fn attest(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let input: Mutation<data::AttestSubmit> = decode(&body)?;
    let expected = input.command.clone();
    let result = match s
        .runtime
        .request(Command::PrepareInvestigationAttestation {
            identity: identity.clone(),
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::InvestigationPreflight(data::Preflight::Recorded(value)) => value,
        Reply::InvestigationPreflight(data::Preflight::Verify(ticket)) => {
            let prepared = worker(&s)?
                .prepare_attestation(*ticket)
                .await
                .map_err(verification_error)?;
            match s
                .runtime
                .request(Command::CommitInvestigationAttestation(Box::new(prepared)))
                .await?
            {
                Reply::InvestigationAttestation(value) => value,
                _ => return Err(mismatch()),
            }
        }
        _ => return Err(mismatch()),
    };
    if result.actor.principal != identity.principal
        || !same_attestation_request(&result.request, &expected)
    {
        return Err(mismatch());
    }
    let expected_reference = result.reference().map_err(|_| mismatch())?;
    let response = attestation_response(
        read_attestation(&s, identity, expected.operation, result.id.clone()).await?,
    )?;
    if response.reference != expected_reference {
        return Err(mismatch());
    }
    Ok(Json(response).into_response())
}
pub(super) async fn get_attestation(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<RecordQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    Ok(Json(attestation_response(
        read_attestation(&s, identity, input.operation, input.id).await?,
    )?)
    .into_response())
}
pub(super) async fn list_attestations(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    if !(1..=50).contains(&input.limit) {
        return Err(ApiError::invalid());
    }
    let Reply::InvestigationAttestationPage(page) = s
        .runtime
        .request(Command::ListInvestigationAttestations {
            identity,
            operation: input.operation.clone(),
            after: input.after,
            limit: input.limit,
        })
        .await?
    else {
        return Err(mismatch());
    };
    if page.items.len() > input.limit
        || page
            .items
            .iter()
            .any(|v| v.attestation.request.operation != input.operation)
    {
        return Err(mismatch());
    }
    let items = page
        .items
        .into_iter()
        .map(attestation_response)
        .collect::<Result<_, _>>()?;
    Ok(Json(AttestationPageResponse {
        items,
        next: page.next,
        operation_authorized: false,
        resource_release_authorized: false,
    })
    .into_response())
}
pub(super) async fn record_disposition(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let input: Mutation<data::RecordDisposition> = decode(&body)?;
    let expected = input.command.clone();
    let result = match s
        .runtime
        .request(Command::PrepareInvestigationDisposition {
            identity: identity.clone(),
            key: input.request_key,
            input: input.command,
        })
        .await?
    {
        Reply::InvestigationDispositionPreflight(data::DispositionPreflight::Recorded(value)) => {
            value
        }
        Reply::InvestigationDispositionPreflight(data::DispositionPreflight::Verify(ticket)) => {
            let prepared = worker(&s)?
                .prepare_disposition(*ticket)
                .await
                .map_err(verification_error)?;
            match s
                .runtime
                .request(Command::CommitInvestigationDisposition(Box::new(prepared)))
                .await?
            {
                Reply::InvestigationReceipt(value) => value,
                _ => return Err(mismatch()),
            }
        }
        _ => return Err(mismatch()),
    };
    if result.actor.principal != identity.principal
        || !same_disposition_request(&result.request, &expected)
    {
        return Err(mismatch());
    }
    let expected_reference = result.reference().map_err(|_| mismatch())?;
    let response = disposition_response(
        read_disposition(&s, identity, expected.operation, result.id.clone()).await?,
    )?;
    if response.reference != expected_reference {
        return Err(mismatch());
    }
    Ok(Json(response).into_response())
}
pub(super) async fn get_disposition(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<RecordQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    Ok(Json(disposition_response(
        read_disposition(&s, identity, input.operation, input.id).await?,
    )?)
    .into_response())
}
pub(super) async fn list_dispositions(
    State(s): State<ApiState>,
    headers: HeaderMap,
    input: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let identity = investigation_identity(&s, &headers).await?;
    let Query(input) = input.map_err(|_| ApiError::invalid())?;
    if !(1..=50).contains(&input.limit) {
        return Err(ApiError::invalid());
    }
    let Reply::InvestigationReceiptPage(page) = s
        .runtime
        .request(Command::ListInvestigationDispositions {
            identity,
            operation: input.operation.clone(),
            after: input.after,
            limit: input.limit,
        })
        .await?
    else {
        return Err(mismatch());
    };
    if page.items.len() > input.limit
        || page
            .items
            .iter()
            .any(|v| v.receipt.request.operation != input.operation)
    {
        return Err(mismatch());
    }
    let items = page
        .items
        .into_iter()
        .map(disposition_response)
        .collect::<Result<_, _>>()?;
    Ok(Json(DispositionPageResponse {
        items,
        next: page.next,
        operation_authorized: false,
        resource_release_authorized: false,
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_writer_failures_have_fixed_codes_and_no_unknown_commit_claim() {
        use rx_runtime::investigation::Error;
        for (error, status, code) in [
            (Error::Busy, StatusCode::TOO_MANY_REQUESTS, "INVESTIGATION_BUSY"),
            (Error::Unavailable, StatusCode::SERVICE_UNAVAILABLE, "INVESTIGATION_UNAVAILABLE"),
            (Error::VerificationFailed, StatusCode::CONFLICT, "INVESTIGATION_VERIFICATION_FAILED"),
        ] {
            let response = verification_error(error);
            assert_eq!(response.status, status);
            assert_eq!(response.code, code);
            assert!(!response.outcome_unknown);
        }
    }
}
