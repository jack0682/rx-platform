use super::*;
use rx_ports::{OutboxRecord, OutboxState};
const PAGE: usize = 128;

pub(super) fn candidates<R: Repository>(repository: &mut R) -> Result<Vec<Id>> {
    let mut after = None;
    let mut ids = Vec::new();
    loop {
        let page = repository.pending_outbox_after(after.as_ref(), PAGE)?;
        if page.len() > PAGE || ids.len() + page.len() > r::MAX_PENDING_SCAN {
            return Err(StoreError::Invalid(
                "HOST_RECOVERY_PENDING_SCAN_OVERFLOW: complete candidate coverage required".into(),
            ));
        }
        let mut previous = after.clone();
        for row in &page {
            if previous.as_ref().is_some_and(|last| row.id <= *last) {
                return Err(StoreError::Integrity(
                    "pending outbox page is not strictly ordered".into(),
                ));
            }
            previous = Some(row.id.clone());
            ids.push(row.id.clone());
        }
        if page.len() < PAGE {
            return Ok(ids);
        }
        after = previous;
    }
}
fn matches(row: &OutboxRecord, host: &Name, task: &r::FenceTask) -> Result<bool> {
    if row.document.schema.as_str() != DELIVERY {
        return Ok(false);
    }
    let payload: Delivery =
        canonical::decode_json(&canonical::bytes(&row.document.value).map_err(domain_error)?)
            .map_err(domain_error)?;
    Ok(
        matches!(payload, Delivery::Fence {cell, host: target, epoch, scopes, block_ids}
        if cell == task.cell && target == *host && epoch == task.epoch && scopes == task.scopes && block_ids == task.block_ids),
    )
}
pub(super) fn matching(
    tx: &mut dyn Transaction,
    host: &Name,
    task: &r::FenceTask,
    candidates: &[Id],
) -> Result<Vec<Id>> {
    let mut selected = Vec::new();
    for id in candidates {
        if let Some(row) = tx.outbox(id)?
            && matches!(row.state, OutboxState::New | OutboxState::EmitEntered)
            && matches(&row, host, task)?
        {
            selected.push(id.clone());
        }
    }
    Ok(selected)
}
fn original(
    tx: &mut dyn Transaction,
    host: &Name,
    task: &r::FenceTask,
) -> Result<Option<OutboxRecord>> {
    let Some(message) = &task.originating_message else {
        return Ok(None);
    };
    let row = tx
        .outbox(message)?
        .ok_or_else(|| StoreError::Integrity("original recovery Fence outbox missing".into()))?;
    if message != &task.request || row.id != *message || !matches(&row, host, task)? {
        return Err(StoreError::Integrity(
            "original recovery Fence ID/body differs".into(),
        ));
    }
    Ok(Some(row))
}
pub(super) fn enter(tx: &mut dyn Transaction, host: &Name, task: &r::FenceTask) -> Result<()> {
    if let Some(row) = original(tx, host, task)? {
        match row.state {
            OutboxState::New => {
                tx.transition_outbox(&row.id, OutboxState::New, OutboxState::EmitEntered)?
            }
            OutboxState::EmitEntered | OutboxState::Delivered => {}
            OutboxState::Voided => return reject(Reject::ContinuityUnproven),
        }
    }
    Ok(())
}
pub(super) fn complete(tx: &mut dyn Transaction, host: &Name, task: &r::FenceTask) -> Result<()> {
    if let Some(row) = original(tx, host, task)? {
        match row.state {
            OutboxState::EmitEntered => {
                tx.transition_outbox(&row.id, OutboxState::EmitEntered, OutboxState::Delivered)?
            }
            OutboxState::Delivered => {}
            _ => return reject(Reject::ContinuityUnproven),
        }
    }
    Ok(())
}
