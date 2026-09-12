use super::*;
use crate::runtime_invalidation;

/// The current P boot must already restrict every registered cell of this producer.
/// A reason string, an older boot's origin, or a changed configuration is insufficient.
pub(super) fn covers_registrations(
    tx: &mut dyn Transaction,
    meta: &Installation,
    principal: &Name,
    previous_boot: &Id,
    registrations: &[HostRegistration],
) -> Result<bool> {
    let cells: BTreeSet<_> = registrations
        .iter()
        .filter(|host| &host.id == principal)
        .map(|host| &host.cell)
        .collect();
    if cells.is_empty() {
        return Ok(false);
    }
    for cell_id in cells {
        let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
        let configuration = runtime_invalidation::configuration_digest(&cell.configuration)
            .map_err(StoreError::Integrity)?;
        let mut covered = false;
        for block in cell
            .blocks
            .iter()
            .filter(|block| block.latched && block.reason == BlockReason::RuntimeRestart)
        {
            let Some(origin) = runtime_invalidation::read_for_cell(tx, meta, cell_id, &block.id)?
            else {
                continue;
            };
            if origin.runtime_boot == meta.runtime_boot
                && &origin.previous_runtime_boot == previous_boot
                && origin.after.configuration_digest == configuration
                && origin.after.scope_epochs.keys().collect::<BTreeSet<_>>()
                    == cell.scope_epochs.keys().collect()
            {
                covered = true;
            }
        }
        if !covered {
            return Ok(false);
        }
    }
    Ok(true)
}
