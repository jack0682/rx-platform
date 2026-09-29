//! Immutable origin of a restriction the platform runtime itself raised: the RuntimeRestart block
//! a new runtime adds to every cell, or the AuthorityRevoked block a runtime stop request adds.
//! Provenance is not permission to clear it.
use crate::{Block, BlockReason, Cell, CellConfiguration, Installation, persistence};
use rx_domain::{canonical, types::*};
use rx_ports::{Result, StoreError, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "rx.runtime-invalidation-origin.v1";
const PREFIX: &str = "runtimeinvalidationorigin";
const CELL: &str = "rx.internal.cell.v1";
const INSTALLATION: &str = "rx.internal.installation.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellBoundary {
    pub revision: Counter,
    pub epoch: Counter,
    pub scope_epochs: BTreeMap<Name, Counter>,
    /// SHA-256 of canonical CellConfiguration bytes, matching process-change configuration refs.
    pub configuration_digest: Digest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInvalidationOrigin {
    pub installation: Id,
    pub store_generation: Id,
    pub cell: Name,
    pub block: Block,
    pub previous_runtime_boot: Id,
    pub runtime_boot: Id,
    pub before: CellBoundary,
    pub after: CellBoundary,
    /// Set when the stop request `stop` of the runtime raised this AuthorityRevoked block; both
    /// boots then name that runtime. Absent for RuntimeRestart, whose bytes stay unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRestriction {
    pub block: Block,
    pub origin: Option<RuntimeInvalidationOrigin>,
    pub origin_digest: Option<Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRestrictions {
    pub cell: Name,
    pub revision: Counter,
    pub epoch: Counter,
    pub configuration_digest: Digest,
    pub restrictions: Vec<RuntimeRestriction>,
}
impl RuntimeInvalidationOrigin {
    pub fn validate(&self) -> std::result::Result<(), String> {
        let cause_matches = match self.stop {
            None => {
                self.previous_runtime_boot != self.runtime_boot
                    && self.block.reason == BlockReason::RuntimeRestart
            }
            Some(_) => {
                self.previous_runtime_boot == self.runtime_boot
                    && self.block.reason == BlockReason::AuthorityRevoked
            }
        };
        if !cause_matches
            || self.before.revision.0 == 0
            || self.before.epoch.0 == 0
            || self.before.revision.0.checked_add(1) != Some(self.after.revision.0)
            || self.before.epoch.0.checked_add(1) != Some(self.after.epoch.0)
            || self.before.configuration_digest != self.after.configuration_digest
            || self.before.scope_epochs.is_empty()
            || self.before.scope_epochs.keys().collect::<BTreeSet<_>>()
                != self.after.scope_epochs.keys().collect()
            || self.before.scope_epochs.iter().any(|(scope, epoch)| {
                epoch.0 == 0
                    || epoch.0.checked_add(1) != self.after.scope_epochs.get(scope).map(|v| v.0)
            })
            || !self.block.latched
            || self.block.case_id.is_some()
            || self.block.created_revision != Some(self.after.revision)
            || self.block.scopes.len() != self.after.scope_epochs.len()
            || self.block.scopes.iter().collect::<BTreeSet<_>>()
                != self.after.scope_epochs.keys().collect()
        {
            return Err("runtime invalidation origin boundaries or block differ".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> std::result::Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-RUNTIME-INVALIDATION-ORIGIN-v1", self).map_err(|e| e.to_string())
    }
}

pub fn configuration_digest(
    configuration: &CellConfiguration,
) -> std::result::Result<Digest, String> {
    let bytes = canonical::bytes(configuration).map_err(|e| e.to_string())?;
    Ok(rx_package::content_digest(&bytes))
}
pub(crate) fn boundary(revision: Counter, cell: &Cell) -> Result<CellBoundary> {
    Ok(CellBoundary {
        revision,
        epoch: cell.epoch,
        scope_epochs: cell.scope_epochs.clone(),
        configuration_digest: configuration_digest(&cell.configuration)
            .map_err(StoreError::Integrity)?,
    })
}
pub(crate) fn same<T: Serialize>(left: &T, right: &T) -> Result<bool> {
    Ok(
        canonical::bytes(left).map_err(|e| StoreError::Integrity(e.to_string()))?
            == canonical::bytes(right).map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
}

/// Record only the restriction just created by this boot transaction, never an older matching reason.
pub(crate) fn record_restart(
    tx: &mut dyn Transaction,
    previous: &Installation,
    current: &Installation,
    before_revision: Counter,
    before: &Cell,
    after: &Cell,
) -> Result<()> {
    if previous.id != current.id || previous.store_generation != current.store_generation {
        return Err(StoreError::Integrity(
            "runtime invalidation installation or cell changed".into(),
        ));
    }
    let origin = origin(
        tx,
        current,
        &previous.runtime_boot,
        None,
        before_revision,
        before,
        after,
    )?;
    origin.validate().map_err(StoreError::Integrity)?;
    persistence::save(tx, PREFIX, &origin.block.id, None, SCHEMA, &origin)?;
    Ok(())
}

/// Record the AuthorityRevoked block the stop request `stop` just added to `before`'s cell.
/// A cell whose epochs or scopes disagree with its configuration cannot carry well-formed
/// provenance; its block still stands, keeps no origin and so has no release path. Recording
/// never keeps the runtime from stopping.
pub(crate) fn record_stop(
    tx: &mut dyn Transaction,
    current: &Installation,
    stop: &Id,
    before_revision: Counter,
    before: &Cell,
) -> Result<()> {
    let (_, after): (_, Cell) = persistence::load(tx, "cell", &before.configuration.id, CELL)?;
    let origin = origin(
        tx,
        current,
        &current.runtime_boot,
        Some(stop.clone()),
        before_revision,
        before,
        &after,
    )?;
    if origin.validate().is_ok() {
        persistence::save(tx, PREFIX, &origin.block.id, None, SCHEMA, &origin)?;
    }
    Ok(())
}

/// The origin of the one block `after` adds to `before`, as stored in this transaction.
fn origin(
    tx: &mut dyn Transaction,
    current: &Installation,
    previous_runtime_boot: &Id,
    stop: Option<Id>,
    before_revision: Counter,
    before: &Cell,
    after: &Cell,
) -> Result<RuntimeInvalidationOrigin> {
    if before.configuration.id != after.configuration.id {
        return Err(StoreError::Integrity(
            "runtime invalidation installation or cell changed".into(),
        ));
    }
    let before_ids: BTreeSet<_> = before.blocks.iter().map(|b| &b.id).collect();
    let after_ids: BTreeSet<_> = after.blocks.iter().map(|b| &b.id).collect();
    if before_ids.len() != before.blocks.len()
        || after_ids.len() != after.blocks.len()
        || after.blocks.len() != before.blocks.len() + 1
        || !before_ids.is_subset(&after_ids)
    {
        return Err(StoreError::Integrity(
            "runtime invalidation must add exactly one block".into(),
        ));
    }
    for block in &before.blocks {
        let preserved = after
            .blocks
            .iter()
            .find(|b| b.id == block.id)
            .ok_or_else(|| StoreError::Integrity("prior runtime block disappeared".into()))?;
        if !same(block, preserved)? {
            return Err(StoreError::Integrity("prior runtime block changed".into()));
        }
    }
    let block = after
        .blocks
        .iter()
        .find(|b| !before_ids.contains(&b.id))
        .ok_or_else(|| StoreError::Integrity("new runtime block absent".into()))?;
    let (after_revision, stored): (_, Cell) =
        persistence::load(tx, "cell", &after.configuration.id, CELL)?;
    if !same(&stored, after)? {
        return Err(StoreError::Integrity(
            "runtime invalidation cell was not stored at this boundary".into(),
        ));
    }
    Ok(RuntimeInvalidationOrigin {
        installation: current.id.clone(),
        store_generation: current.store_generation.clone(),
        cell: after.configuration.id.clone(),
        block: block.clone(),
        previous_runtime_boot: previous_runtime_boot.clone(),
        runtime_boot: current.runtime_boot.clone(),
        before: boundary(before_revision, before)?,
        after: boundary(after_revision, after)?,
        stop,
    })
}

/// Whether the current runtime already restricts every cell in `cells` with a RuntimeRestart
/// block whose origin ends exactly `previous_boot`, for the current configuration and the whole
/// scope vector. A peer whose last session belonged to `previous_boot` then reconnects after a
/// P-only restart: its authority was already revoked, with provenance, by that restriction. A
/// reason string, an older boot's origin or a changed configuration is insufficient.
pub(crate) fn restart_covers<'a>(
    tx: &mut dyn Transaction,
    meta: &Installation,
    previous_boot: &Id,
    cells: impl IntoIterator<Item = &'a Name>,
) -> Result<bool> {
    let mut any = false;
    for cell_id in cells {
        any = true;
        let (_, cell): (_, Cell) = persistence::load(tx, "cell", cell_id, CELL)?;
        let configuration =
            configuration_digest(&cell.configuration).map_err(StoreError::Integrity)?;
        let mut covered = false;
        for block in cell
            .blocks
            .iter()
            .filter(|block| block.latched && block.reason == BlockReason::RuntimeRestart)
        {
            let Some(origin) = read_for_cell(tx, meta, cell_id, &block.id)? else {
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
    Ok(any)
}

/// Read a historical origin. Absence stays absence; a reason string never synthesizes provenance.
pub fn load(tx: &mut dyn Transaction, block_id: &Id) -> Result<Option<RuntimeInvalidationOrigin>> {
    let key = persistence::key(PREFIX, block_id);
    let Some(record) = tx.get(&key)? else {
        return Ok(None);
    };
    let origin: RuntimeInvalidationOrigin = persistence::decode(&record, SCHEMA)?;
    if record.key != key || record.revision != Counter(1) || origin.block.id != *block_id {
        return Err(StoreError::Integrity(
            "runtime invalidation origin key or immutable revision differs".into(),
        ));
    }
    origin.validate().map_err(StoreError::Integrity)?;
    Ok(Some(origin))
}

/// Read provenance for the exact restriction still present in this cell at the current store cut.
/// Earlier boot origins remain readable. Current configuration/acceptance-plan eligibility, Host
/// continuity, ownership and any removal authorization remain the caller's separate responsibility.
pub fn read_for_cell(
    tx: &mut dyn Transaction,
    installation: &Installation,
    cell_id: &Name,
    block_id: &Id,
) -> Result<Option<RuntimeInvalidationOrigin>> {
    let Some(origin) = load(tx, block_id)? else {
        return Ok(None);
    };
    let record = tx
        .get(&persistence::name("installation/current"))?
        .ok_or_else(|| StoreError::Integrity("runtime installation absent".into()))?;
    let live: Installation = persistence::decode(&record, INSTALLATION)?;
    // An origin recorded before a restore, in the generation the restored cut was written
    // under, is still this ledger's history; any other generation is not.
    if !same(&live, installation)?
        || origin.installation != installation.id
        || !crate::engine::store_restore::lineage(tx, &installation.store_generation)?
            .contains(&origin.store_generation)
        || origin.cell != *cell_id
    {
        return Err(StoreError::Integrity(
            "runtime invalidation origin belongs to another installation, generation or cell"
                .into(),
        ));
    }
    let (revision, cell): (_, Cell) = persistence::load(tx, "cell", cell_id, CELL)?;
    let blocks: Vec<_> = cell.blocks.iter().filter(|b| b.id == *block_id).collect();
    if cell.configuration.id != *cell_id
        || blocks.len() != 1
        || !same(blocks[0], &origin.block)?
        || revision < origin.after.revision
        || cell.epoch < origin.after.epoch
        || origin.after.scope_epochs.iter().any(|(scope, value)| {
            cell.scope_epochs
                .get(scope)
                .is_none_or(|current| current < value)
        })
    {
        return Err(StoreError::Integrity(
            "runtime invalidation origin no longer matches the exact cell block or boundaries"
                .into(),
        ));
    }
    Ok(Some(origin))
}
