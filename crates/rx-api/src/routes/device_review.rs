use super::*;
use rx_application::device_review as review;
pub(super) async fn create(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<review::Create> = decode(&body)?;
    match s
        .runtime
        .request(Command::CreateDeviceReview {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::DeviceReviewJob(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn get(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReviewQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::GetDeviceReview {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            id: q.id,
            revision: q.revision,
        })
        .await?
    {
        Reply::DeviceReviewDetail(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn list(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ReviewListQuery>,
) -> Result<Response, ApiError> {
    match s
        .runtime
        .request(Command::ListDeviceReviews {
            identity: identity(&s, &headers)?,
            cell: q.cell,
            intake: q.intake,
            after: q.after,
        })
        .await?
    {
        Reply::DeviceReviews(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn report(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<review::Submit> = decode(&body)?;
    let t = match s
        .runtime
        .request(Command::PrepareDeviceReport {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::DeviceReportPreflight(review::Preflight::Recorded(v)) => {
            return Ok(Json(v).into_response());
        }
        Reply::DeviceReportPreflight(review::Preflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let worker = s.package_intake.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DEVICE_REVIEW_NOT_CONFIGURED",
        )
    })?;
    let prepared = worker
        .prepare_device_report(*t)
        .await
        .map_err(|_| ApiError::new(StatusCode::CONFLICT, "DEVICE_REPORT_VERIFICATION_FAILED"))?;
    match s
        .runtime
        .request(Command::CommitDeviceReport(Box::new(prepared)))
        .await?
    {
        Reply::DeviceReviewVersion(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
pub(super) async fn decide(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let v: Mutation<review::Decide> = decode(&body)?;
    let t = match s
        .runtime
        .request(Command::PrepareDeviceDecision {
            identity: identity(&s, &headers)?,
            key: v.request_key,
            input: v.command,
        })
        .await?
    {
        Reply::DeviceDecisionPreflight(review::DecisionPreflight::Recorded(v)) => {
            return Ok(Json(v).into_response());
        }
        Reply::DeviceDecisionPreflight(review::DecisionPreflight::Verify(t)) => t,
        _ => return Err(mismatch()),
    };
    let prepared = if t.requires_verification() {
        let worker = s.package_intake.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "DEVICE_REVIEW_NOT_CONFIGURED",
            )
        })?;
        worker.prepare_device_decision(*t).await.map_err(|_| {
            ApiError::new(
                StatusCode::CONFLICT,
                "DEVICE_APPROVAL_REVERIFICATION_FAILED",
            )
        })?
    } else {
        review::PreparedDecision::reject(*t).map_err(|_| mismatch())?
    };
    match s
        .runtime
        .request(Command::CommitDeviceDecision(Box::new(prepared)))
        .await?
    {
        Reply::DeviceReviewDecision(v) => Ok(Json(v).into_response()),
        _ => Err(mismatch()),
    }
}
