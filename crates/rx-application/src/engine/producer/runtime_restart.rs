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
    runtime_invalidation::restart_covers(tx, meta, previous_boot, cells)
}
