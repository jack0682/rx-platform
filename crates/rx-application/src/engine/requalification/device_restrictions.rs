use super::*;
use crate::device_invalidation::{
    self as origin, DeviceInvalidationCause, DeviceInvalidationOrigin,
};

pub(super) fn select(
    tx: &mut dyn Transaction,
    meta: &Installation,
    input: &q::Begin,
    change: &crate::process_change::Change,
) -> Result<Vec<DeviceInvalidationOrigin>> {
    if input.device_restrictions.len() > 128 {
        return reject(Reject::InvalidInput);
    }
    let application = change
        .application
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let mut selected = Vec::new();
    for (block, expected) in &input.device_restrictions {
        let historical =
            origin::load(tx, block)?.ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if !input.expected_cells.contains_key(&historical.cell) {
            return reject(Reject::Forbidden);
        }
        let actual = origin::read_for_cell(tx, meta, &historical.cell, block)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if actual.digest().map_err(StoreError::Integrity)? != *expected
            || !application.cells.iter().any(|c| {
                c.cell == actual.cell
                    && [c.before.sha256, c.after.sha256]
                        .contains(&actual.after.configuration_digest)
            })
        {
            return reject(Reject::StaleRevision);
        }
        // Provenance is not permission: a device restriction is releasable only once the Host
        // has actually been re-linked after the invalidation.
        relinked(tx, &actual)?;
        selected.push(actual);
    }
    Ok(selected)
}

pub(super) fn current(tx: &mut dyn Transaction, meta: &Installation, job: &q::Job) -> Result<()> {
    for expected in &job.request.device_restrictions {
        let actual = origin::read_for_cell(tx, meta, &expected.cell, &expected.block.id)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if actual.digest().map_err(StoreError::Integrity)?
            != expected.digest().map_err(StoreError::Integrity)?
        {
            return reject(Reject::StaleRevision);
        }
        relinked(tx, &actual)?;
    }
    Ok(())
}

/// The release precondition: the Host's current registration for the registration cell belongs
/// to the generation of a host link committed at or after the epoch this invalidation left that
/// cell at, and it has moved past what the invalidation named. Only the link's epoch is compared:
/// fence acknowledgements, including this requalification's own, advance the registration's
/// epoch within the same generation. A restart alone, or a registration from before the
/// invalidation, never proves re-link.
fn relinked(tx: &mut dyn Transaction, origin: &DeviceInvalidationOrigin) -> Result<()> {
    let row = tx
        .get(&key("host", (&origin.registration_cell, &origin.host)))?
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    let registration: HostRegistration = decode(&row, HOST)?;
    let plan = host_link::current_bound(tx, &origin.host, &origin.registration_cell)?
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    if plan.epoch < origin.registration_epoch
        || plan.host_boot != registration.boot_id
        || plan.producer_session != registration.session
        || plan.delivery_journal != registration.delivery_journal
        || plan.source_sessions != registration.source_sessions
    {
        return reject(Reject::ContinuityUnproven);
    }
    let advanced = match &origin.cause {
        DeviceInvalidationCause::ProducerReplaced {
            previous_boot,
            previous_session,
            ..
        } => registration.boot_id != *previous_boot || registration.session != *previous_session,
        // The link read the source afresh; its generation is the new baseline, whatever it is.
        DeviceInvalidationCause::SourceGenerationChanged { source, .. } => {
            registration.source_sessions.contains_key(source)
        }
    };
    if !advanced {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}
