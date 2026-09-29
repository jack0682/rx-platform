//! Immutable provenance of an authenticated Host incarnation replacement.
//! A matching origin is neither a device continuity proof nor clearance authority.
use crate::{persistence as p, runtime_invalidation as runtime, *};
use rx_domain::{canonical, types::*};
use rx_ports::{Result, StoreError, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "rx.host-invalidation-origin.v1";
const PREFIX: &str = "host-invalidation-origin";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerIdentity {
    pub principal: Name,
    pub session: Id,
    pub boot: Id,
    pub journal: Id,
    pub authentication_binding: Digest,
}
impl From<&EvidenceProducer> for ProducerIdentity {
    fn from(p: &EvidenceProducer) -> Self {
        Self {
            principal: p.principal.clone(),
            session: p.session.clone(),
            boot: p.peer_boot.clone(),
            journal: p.journal.clone(),
            authentication_binding: p.authentication_binding,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellChange {
    pub before: runtime::CellBoundary,
    pub after: runtime::CellBoundary,
    pub block: Block,
    pub prior_block_ids: BTreeSet<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub installation: Id,
    pub store_generation: Id,
    pub previous_runtime_boot: Id,
    pub runtime_boot: Id,
    pub before: ProducerIdentity,
    pub after: ProducerIdentity,
    pub registrations: BTreeMap<Name, HostRegistration>,
    pub cells: BTreeMap<Name, CellChange>,
}
impl Origin {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.before.principal != self.after.principal
            || self.before.boot == self.after.boot
            || self.before.session == self.after.session
            || self.cells.is_empty()
            || self.registrations.is_empty()
            || self.registrations.iter().any(|(id, r)| {
                r.id != self.before.principal || &r.cell != id || !self.cells.contains_key(id)
            })
        {
            return Err("Host replacement identity/cohort differs".into());
        }
        let mut blocks = BTreeSet::new();
        for cut in self.cells.values() {
            if cut.before.revision.0 == 0
                || cut.before.epoch.0 == 0
                || cut.before.revision.0.checked_add(1) != Some(cut.after.revision.0)
                || cut.before.epoch.0.checked_add(1) != Some(cut.after.epoch.0)
                || cut.before.configuration_digest != cut.after.configuration_digest
                || cut.before.scope_epochs.is_empty()
                || cut.before.scope_epochs.keys().collect::<BTreeSet<_>>()
                    != cut.after.scope_epochs.keys().collect()
                || cut
                    .before
                    .scope_epochs
                    .iter()
                    .any(|(s, n)| n.0.checked_add(1) != cut.after.scope_epochs.get(s).map(|n| n.0))
                || cut.block.reason != BlockReason::DeviceRestart
                || !cut.block.latched
                || cut.block.case_id.is_some()
                || cut.block.created_revision != Some(cut.after.revision)
                || cut.block.scopes.len() != cut.after.scope_epochs.len()
                || cut.block.scopes.iter().collect::<BTreeSet<_>>()
                    != cut.after.scope_epochs.keys().collect()
                || cut.prior_block_ids.contains(&cut.block.id)
                || !blocks.insert(&cut.block.id)
            {
                return Err("Host replacement cell boundary or block differs".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> std::result::Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-INVALIDATION-ORIGIN-v1", self).map_err(|e| e.to_string())
    }
}
fn same<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(
        canonical::bytes(a).map_err(|e| StoreError::Integrity(e.to_string()))?
            == canonical::bytes(b).map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
}
pub(crate) struct Replacement<'a> {
    pub previous: &'a EvidenceProducer,
    pub previous_runtime: &'a Id,
    pub current: &'a EvidenceProducer,
}
pub(crate) fn record(
    tx: &mut dyn Transaction,
    meta: &Installation,
    replacement: Replacement<'_>,
    registrations: &[HostRegistration],
    previous_cells: &BTreeMap<Name, (Counter, Cell)>,
    touched: &BTreeSet<Name>,
) -> Result<()> {
    let Replacement {
        previous: before,
        previous_runtime: previous_runtime_boot,
        current: after,
    } = replacement;
    let mut cells = BTreeMap::new();
    for id in touched {
        let (revision, old) = previous_cells.get(id).ok_or(StoreError::Integrity(
            "Host invalidation before-cut missing".into(),
        ))?;
        let (current_revision, current): (_, Cell) =
            p::load(tx, "cell", id, "rx.internal.cell.v1")?;
        let prior: BTreeSet<_> = old.blocks.iter().map(|b| b.id.clone()).collect();
        if prior.len() != old.blocks.len() || current.blocks.len() != old.blocks.len() + 1 {
            return Err(StoreError::Integrity(
                "Host invalidation changed prior restrictions".into(),
            ));
        }
        for block in &old.blocks {
            let kept = current
                .blocks
                .iter()
                .find(|b| b.id == block.id)
                .ok_or(StoreError::Integrity("prior restriction missing".into()))?;
            if !same(block, kept)? {
                return Err(StoreError::Integrity("prior restriction changed".into()));
            }
        }
        let added: Vec<_> = current
            .blocks
            .iter()
            .filter(|b| !prior.contains(&b.id))
            .collect();
        if added.len() != 1 {
            return Err(StoreError::Integrity(
                "Host invalidation needs one exact new restriction".into(),
            ));
        }
        cells.insert(
            id.clone(),
            CellChange {
                before: runtime::boundary(*revision, old)?,
                after: runtime::boundary(current_revision, &current)?,
                block: added[0].clone(),
                prior_block_ids: prior,
            },
        );
    }
    let value = Origin {
        installation: meta.id.clone(),
        store_generation: meta.store_generation.clone(),
        previous_runtime_boot: previous_runtime_boot.clone(),
        runtime_boot: meta.runtime_boot.clone(),
        before: before.into(),
        after: after.into(),
        registrations: registrations
            .iter()
            .filter(|r| r.id == before.principal && touched.contains(&r.cell))
            .map(|r| (r.cell.clone(), r.clone()))
            .collect(),
        cells,
    };
    value.validate().map_err(StoreError::Integrity)?;
    p::save(tx, PREFIX, &after.session, None, SCHEMA, &value)?;
    Ok(())
}
pub fn load(tx: &mut dyn Transaction, session: &Id) -> Result<Option<Origin>> {
    let key = p::key(PREFIX, session);
    let Some(row) = tx.get(&key)? else {
        return Ok(None);
    };
    let value: Origin = p::decode(&row, SCHEMA)?;
    if row.key != key || row.revision != Counter(1) || value.after.session != *session {
        return Err(StoreError::Integrity(
            "Host invalidation key or immutable revision differs".into(),
        ));
    }
    value.validate().map_err(StoreError::Integrity)?;
    Ok(Some(value))
}
