use super::*;
use crate::runtime_invalidation::{self as origin, RuntimeInvalidationOrigin};

pub(super) fn select(
    tx: &mut dyn Transaction,
    meta: &Installation,
    input: &q::Begin,
    change: &crate::process_change::Change,
) -> Result<Vec<RuntimeInvalidationOrigin>> {
    if input.runtime_restrictions.len() > 128 {
        return reject(Reject::InvalidInput);
    }
    let application = change
        .application
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let mut selected = Vec::new();
    for (block, expected) in &input.runtime_restrictions {
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
        selected.push(actual);
    }
    Ok(selected)
}

pub(super) fn current(tx: &mut dyn Transaction, meta: &Installation, job: &q::Job) -> Result<()> {
    for expected in &job.request.runtime_restrictions {
        let actual = origin::read_for_cell(tx, meta, &expected.cell, &expected.block.id)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if actual.digest().map_err(StoreError::Integrity)?
            != expected.digest().map_err(StoreError::Integrity)?
        {
            return reject(Reject::StaleRevision);
        }
    }
    Ok(())
}
