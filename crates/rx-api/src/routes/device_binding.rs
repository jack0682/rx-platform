use super::*;
use rx_application::device_binding as binding;
pub(super) async fn propose(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<binding::Propose> = decode(&body)?;
    let ticket = match s
        .runtime
        .request(Command::PrepareDeviceBinding {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::DeviceBindingPreflight(binding::Preflight::Recorded(p)) => {
            return Ok(Json(p).into_response());
        }
        Reply::DeviceBindingPreflight(binding::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DEVICE_REVIEW_NOT_CONFIGURED",
        )
    })?;
    let prepared = worker
        .prepare_device_binding(*ticket)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "DEVICE_BINDING_REVERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitDeviceBinding(Box::new(prepared)))
        .await?
    {
        Reply::DeviceBindingPlan(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn review(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<binding::ReviewImpact> = decode(&body)?;
    let ticket = match s
        .runtime
        .request(Command::PrepareDeviceBindingReview {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::DeviceBindingPreflight(binding::Preflight::Recorded(p)) => {
            return Ok(Json(p).into_response());
        }
        Reply::DeviceBindingPreflight(binding::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DEVICE_REVIEW_NOT_CONFIGURED",
        )
    })?;
    let prepared = worker
        .prepare_device_binding(*ticket)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "DEVICE_BINDING_REVERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitDeviceBindingReview(Box::new(prepared)))
        .await?
    {
        Reply::DeviceBindingPlan(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<IntakeQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetDeviceBindingPlan {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
        })
        .await?
    {
        Reply::DeviceBindingDetail(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn list(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ProcessDraftListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListDeviceBindingPlans {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            after: q.after,
        })
        .await?
    {
        Reply::DeviceBindingPlans(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
