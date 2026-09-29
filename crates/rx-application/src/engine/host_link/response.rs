use super::*;
pub(in crate::engine) fn validate_response(
    meta: &Installation,
    now: &TimePoint,
    plan: &Plan,
    request: &Commit,
) -> Result<()> {
    let ack = &request.fence_receipt;
    if ack.cell != plan.cell
        || ack.invalidation != plan.fence_request
        || ack.host_boot != plan.host_boot
        || ack.journal != plan.delivery_journal
        || ack.epoch != plan.epoch
        || ack.scopes != plan.scopes
        || ack.sequence.0 == 0
    {
        return reject(Reject::ContinuityUnproven);
    }
    let expected_until = request
        .grant_sent_at
        .ticks_ns
        .0
        .checked_add(
            plan.ttl_ms
                .0
                .checked_mul(1_000_000)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
        )
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let grant = &request.grant;
    if grant.owner.as_str() != meta.id.as_str()
        || grant.fence != plan.fence
        || grant.resources != plan.resources
        || grant.ttl_ms != plan.ttl_ms
        || request.grant_sent_at.clock_id != now.clock_id
        || request.grant_sent_at.ticks_ns != plan.prepared_at.ticks_ns
        || request.grant_sent_at.ticks_ns > now.ticks_ns
        || grant.valid_until.clock_id != now.clock_id
        || grant.valid_until.ticks_ns.0 != expected_until
        || grant.valid_until.ticks_ns <= now.ticks_ns
    {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}
