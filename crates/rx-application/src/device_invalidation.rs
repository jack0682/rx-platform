//! Immutable origin of a DeviceRestart restriction. Provenance is not permission to clear it.
//!
//! A DeviceRestart block is raised when a Host's evidence producer is replaced without a proven
//! P-only restart, or when a source generation is lost. This record fixes *that the platform
//! created the restriction* and the exact generation being replaced, so an explicit
//! requalification can later select it for release once the Host has been re-linked. It never
//! synthesizes provenance for a legacy DeviceRestart block, and reading it clears nothing.
use crate::runtime_invalidation::{CellBoundary, boundary, same};
use crate::{Block, BlockReason, Cell, Installation, persistence};
use rx_domain::{canonical, types::*};
use rx_ports::{Result, StoreError, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: &str = "rx.device-invalidation-origin.v1";
const PREFIX: &str = "deviceinvalidationorigin";
const CELL: &str = "rx.internal.cell.v1";
const INSTALLATION: &str = "rx.internal.installation.v1";

/// Why the platform raised this DeviceRestart. Every field names the *replaced* generation, so a
/// later re-link that changes the registration is what proves continuity, not the reason name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum DeviceInvalidationCause {
    /// A Host evidence producer session was replaced and the replacement was not a proven
    /// P-only restart. The previous boot/session are the registration being superseded.
    ProducerReplaced {
        previous_boot: Id,
        previous_session: Id,
        boot: Id,
        session: Id,
    },
    /// A reported source generation differed from the registered one for this fact.
    SourceGenerationChanged {
        source: Name,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_generation: Option<Id>,
        generation: Id,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInvalidationOrigin {
    pub installation: Id,
    pub store_generation: Id,
    pub cell: Name,
    pub host: Name,
    /// Cell of the Host registration this invalidation superseded. A neighbouring cell reached
    /// through a shared scope or resource is released by re-linking the Host there, not here.
    pub registration_cell: Name,
    /// Epoch of `registration_cell` right after this invalidation. Registrations carry the cell
    /// epoch they were written at, so only a link committed at or after it is a re-link.
    pub registration_epoch: Counter,
    pub block: Block,
    pub cause: DeviceInvalidationCause,
    pub before: CellBoundary,
    pub after: CellBoundary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRestriction {
    pub block: Block,
    pub origin: Option<DeviceInvalidationOrigin>,
    pub origin_digest: Option<Digest>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRestrictions {
    pub cell: Name,
    pub revision: Counter,
    pub epoch: Counter,
    pub configuration_digest: Digest,
    pub restrictions: Vec<DeviceRestriction>,
}
impl DeviceInvalidationOrigin {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.before.revision.0 == 0
            || self.before.epoch.0 == 0
            || self.registration_epoch.0 == 0
            || (self.registration_cell == self.cell && self.registration_epoch != self.after.epoch)
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
            || self.block.reason != BlockReason::DeviceRestart
            || !self.block.latched
            || self.block.case_id.is_some()
            || self.block.created_revision != Some(self.after.revision)
            || self.block.scopes.len() != self.after.scope_epochs.len()
            || self.block.scopes.iter().collect::<BTreeSet<_>>()
                != self.after.scope_epochs.keys().collect()
        {
            return Err("device invalidation origin boundaries or block differ".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> std::result::Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-DEVICE-INVALIDATION-ORIGIN-v1", self).map_err(|e| e.to_string())
    }
}

fn current_installation(tx: &mut dyn Transaction) -> Result<Installation> {
    let record = tx
        .get(&persistence::name("installation/current"))?
        .ok_or_else(|| StoreError::Integrity("device invalidation installation absent".into()))?;
    persistence::decode(&record, INSTALLATION)
}

/// Record only the DeviceRestart block just created for `before`'s cell by this transaction, one
/// per affected cell, with the exact generation being replaced. Never an older matching reason.
/// Call it after every closure of the event is invalidated: the registration cell's epoch is
/// read here as the epoch the invalidation left it at.
pub(crate) fn record(
    tx: &mut dyn Transaction,
    host: &Name,
    registration_cell: &Name,
    cause: DeviceInvalidationCause,
    before_revision: Counter,
    before: &Cell,
) -> Result<()> {
    let installation = current_installation(tx)?;
    let (after_revision, after): (_, Cell) =
        persistence::load(tx, "cell", &before.configuration.id, CELL)?;
    let (_, registered): (_, Cell) = persistence::load(tx, "cell", registration_cell, CELL)?;
    if after.configuration.id != before.configuration.id {
        return Err(StoreError::Integrity(
            "device invalidation cell identity differs".into(),
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
            "device invalidation must add exactly one block".into(),
        ));
    }
    for block in &before.blocks {
        let preserved = after
            .blocks
            .iter()
            .find(|b| b.id == block.id)
            .ok_or_else(|| StoreError::Integrity("prior device block disappeared".into()))?;
        if !same(block, preserved)? {
            return Err(StoreError::Integrity("prior device block changed".into()));
        }
    }
    let block = after
        .blocks
        .iter()
        .find(|b| !before_ids.contains(&b.id))
        .ok_or_else(|| StoreError::Integrity("new device block absent".into()))?;
    let origin = DeviceInvalidationOrigin {
        installation: installation.id.clone(),
        store_generation: installation.store_generation.clone(),
        cell: after.configuration.id.clone(),
        host: host.clone(),
        registration_cell: registration_cell.clone(),
        registration_epoch: registered.epoch,
        block: block.clone(),
        cause,
        before: boundary(before_revision, before)?,
        after: boundary(after_revision, &after)?,
    };
    // A cell whose epochs or scopes disagree with its configuration cannot carry well-formed
    // provenance. The block still stands; it keeps no origin and so has no release path.
    if origin.validate().is_ok() {
        persistence::save(tx, PREFIX, &origin.block.id, None, SCHEMA, &origin)?;
    }
    Ok(())
}

/// Read a historical origin. Absence stays absence; a reason string never synthesizes provenance.
pub fn load(tx: &mut dyn Transaction, block_id: &Id) -> Result<Option<DeviceInvalidationOrigin>> {
    let key = persistence::key(PREFIX, block_id);
    let Some(record) = tx.get(&key)? else {
        return Ok(None);
    };
    let origin: DeviceInvalidationOrigin = persistence::decode(&record, SCHEMA)?;
    if record.key != key || record.revision != Counter(1) || origin.block.id != *block_id {
        return Err(StoreError::Integrity(
            "device invalidation origin key or immutable revision differs".into(),
        ));
    }
    origin.validate().map_err(StoreError::Integrity)?;
    Ok(Some(origin))
}

/// Read provenance for the exact restriction still present in this cell at the current store cut.
/// Continuity of the Host (re-link), ownership and any removal authorization stay the caller's
/// separate responsibility: provenance is not permission.
pub fn read_for_cell(
    tx: &mut dyn Transaction,
    installation: &Installation,
    cell_id: &Name,
    block_id: &Id,
) -> Result<Option<DeviceInvalidationOrigin>> {
    let Some(origin) = load(tx, block_id)? else {
        return Ok(None);
    };
    let live = current_installation(tx)?;
    // Same lineage rule as runtime origins: a restored cut keeps the history it was written in.
    if !same(&live, installation)?
        || origin.installation != installation.id
        || !crate::engine::store_restore::lineage(tx, &installation.store_generation)?
            .contains(&origin.store_generation)
        || origin.cell != *cell_id
    {
        return Err(StoreError::Integrity(
            "device invalidation origin belongs to another installation, generation or cell".into(),
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
            "device invalidation origin no longer matches the exact cell block or boundaries"
                .into(),
        ));
    }
    Ok(Some(origin))
}
