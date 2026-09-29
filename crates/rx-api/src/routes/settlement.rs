use super::*;
/// The body carries no observations, booleans or execution permission. The Host
/// read path supplies fresh handover evidence after the scoped approval.
pub(super) async fn approve(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    if !s.terminal_tls {
        return Err(ApiError::forbidden());
    }
    let value: Mutation<rx_application::settlement::Approve> = decode(&body)?;
    match s
        .runtime
        .request(Command::ApproveSettlement {
            identity: identity(&s, &headers)?,
            key: value.request_key,
            command: value.command,
        })
        .await?
    {
        Reply::Settlement(value) => Ok(Json(value).into_response()),
        _ => Err(mismatch()),
    }
}
