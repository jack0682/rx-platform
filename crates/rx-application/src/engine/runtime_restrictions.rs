use super::*;
use crate::runtime_invalidation::{self, RuntimeRestriction, RuntimeRestrictions};

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Read-only selection context; no ownership, block removal or qualification is created.
    pub fn runtime_restrictions(
        &mut self,
        identity: &Identity,
        cell_id: &Name,
    ) -> Result<RuntimeRestrictions> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            // Check access before reading the cell or its origin records, including absent cells.
            let actor = authorize_read(tx, identity, meta, &clock.now(), cell_id)?;
            if !actor
                .roles
                .iter()
                .any(|r| matches!(r, Role::Engineer | Role::Verifier | Role::ReleaseManager))
            {
                return reject(Reject::Forbidden);
            }
            let record = tx
                .get(&name("installation/current"))?
                .ok_or_else(|| StoreError::Integrity("runtime installation missing".into()))?;
            let current: Installation = decode(&record, "rx.internal.installation.v1")?;
            if canonical::bytes(&current).map_err(domain_error)?
                != canonical::bytes(meta).map_err(domain_error)?
            {
                return Err(StoreError::Integrity(
                    "runtime restriction read belongs to a different installation".into(),
                ));
            }
            let (revision, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            if cell.configuration.id != *cell_id {
                return Err(StoreError::Integrity(
                    "runtime restriction cell identity differs".into(),
                ));
            }
            let mut restrictions = Vec::new();
            let mut ids = BTreeSet::new();
            for block in cell
                .blocks
                .iter()
                .filter(|b| b.latched && b.reason == BlockReason::RuntimeRestart)
            {
                if !ids.insert(&block.id) {
                    return Err(StoreError::Integrity(
                        "duplicate runtime restriction ID".into(),
                    ));
                }
                let origin = runtime_invalidation::read_for_cell(tx, meta, cell_id, &block.id)?;
                let origin_digest = origin
                    .as_ref()
                    .map(|value| value.digest())
                    .transpose()
                    .map_err(StoreError::Integrity)?;
                restrictions.push(RuntimeRestriction {
                    block: block.clone(),
                    origin,
                    origin_digest,
                });
            }
            restrictions.sort_by(|a, b| a.block.id.cmp(&b.block.id));
            Ok(RuntimeRestrictions {
                cell: cell_id.clone(),
                revision,
                epoch: cell.epoch,
                configuration_digest: runtime_invalidation::configuration_digest(
                    &cell.configuration,
                )
                .map_err(StoreError::Integrity)?,
                restrictions,
            })
        })
    }
}
