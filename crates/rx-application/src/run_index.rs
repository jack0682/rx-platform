//! Per-cell set of the runs that are neither completed nor abandoned.
//!
//! Hot paths (the maintained-condition watchdog, executor assignment, invalidation, start) ask
//! for a cell's live runs. Scanning `run/` made each of them grow with every run the
//! installation ever had. The control-journal unit of work keeps this set in step with every
//! run write, and every runtime start rebuilds it from the runs themselves. Callers read the
//! members in `run/` key order and apply their own predicates, so they see exactly what a scan
//! filtered to live runs would return.
use crate::{Run, RunState, persistence};
use rx_domain::types::*;
use rx_ports::{Result, StoreError, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = "rx.internal.run-live-index.v1";
const PREFIX: &str = "runlive";
const RUN: &str = "rx.internal.run.v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveRuns {
    cell: Name,
    runs: BTreeSet<Id>,
}

fn live(run: &Run) -> bool {
    !matches!(run.state, RunState::Completed | RunState::Abandoned)
}

/// Keep the set of `run.cell` in step with this write of `run`.
pub(crate) fn record(tx: &mut dyn Transaction, run: &Run) -> Result<()> {
    let key = persistence::key(PREFIX, &run.cell);
    let old = tx.get(&key)?;
    let mut set = match &old {
        Some(row) => persistence::decode::<LiveRuns>(row, SCHEMA)?,
        None => LiveRuns {
            cell: run.cell.clone(),
            runs: BTreeSet::new(),
        },
    };
    if set.cell != run.cell {
        return Err(StoreError::Integrity("live run index cell differs".into()));
    }
    let changed = if live(run) {
        set.runs.insert(run.id.clone())
    } else {
        set.runs.remove(&run.id)
    };
    if changed {
        tx.put(
            &key,
            old.map(|row| row.revision),
            &persistence::doc(SCHEMA, &set)?,
        )?;
    }
    Ok(())
}

/// The live runs of `cell` with their revisions, in the order a scan of `run/` returns them.
pub(crate) fn live_runs(tx: &mut dyn Transaction, cell: &Name) -> Result<Vec<(Counter, Run)>> {
    let Some(row) = tx.get(&persistence::key(PREFIX, cell))? else {
        return Ok(vec![]);
    };
    let set: LiveRuns = persistence::decode(&row, SCHEMA)?;
    if set.cell != *cell {
        return Err(StoreError::Integrity("live run index cell differs".into()));
    }
    let mut keys: Vec<_> = set
        .runs
        .iter()
        .map(|id| (persistence::key("run", id), id))
        .collect();
    keys.sort();
    let mut runs = Vec::with_capacity(keys.len());
    for (key, id) in keys {
        let row = tx
            .get(&key)?
            .ok_or_else(|| StoreError::Integrity("indexed live run absent".into()))?;
        let run: Run = persistence::decode(&row, RUN)?;
        if run.id != *id || run.cell != *cell || !live(&run) {
            return Err(StoreError::Integrity("live run index differs".into()));
        }
        runs.push((row.revision, run));
    }
    Ok(runs)
}

fn wanted(tx: &mut dyn Transaction) -> Result<BTreeMap<Name, BTreeSet<Id>>> {
    let mut wanted: BTreeMap<Name, BTreeSet<Id>> = BTreeMap::new();
    for row in tx.scan("run/")? {
        let run: Run = persistence::decode(&row, RUN)?;
        if live(&run) {
            wanted.entry(run.cell).or_default().insert(run.id);
        }
    }
    Ok(wanted)
}
fn stored(tx: &mut dyn Transaction) -> Result<BTreeMap<Name, (rx_ports::Record, BTreeSet<Id>)>> {
    let mut stored = BTreeMap::new();
    for row in tx.scan(&format!("{PREFIX}/"))? {
        let set: LiveRuns = persistence::decode(&row, SCHEMA)?;
        if row.key != persistence::key(PREFIX, &set.cell) {
            return Err(StoreError::Integrity("live run index key differs".into()));
        }
        stored.insert(set.cell.clone(), (row, set.runs));
    }
    Ok(stored)
}

/// Whether every set equals the live runs of its cell, recomputed from the runs themselves.
pub(crate) fn verify(tx: &mut dyn Transaction) -> Result<()> {
    let wanted = wanted(tx)?;
    let stored: BTreeMap<_, _> = stored(tx)?
        .into_iter()
        .filter(|(_, (_, runs))| !runs.is_empty())
        .map(|(cell, (_, runs))| (cell, runs))
        .collect();
    if stored != wanted {
        return Err(StoreError::Integrity(
            "live run index differs from the runs".into(),
        ));
    }
    Ok(())
}

/// Rebuild every set from the runs themselves. A runtime start calls it, so a ledger written by
/// a revision that did not keep the index, or kept it differently, is corrected before use.
pub(crate) fn rebuild(tx: &mut dyn Transaction) -> Result<()> {
    let mut wanted = wanted(tx)?;
    let mut stored = stored(tx)?;
    let cells: BTreeSet<_> = wanted.keys().chain(stored.keys()).cloned().collect();
    for cell in cells {
        let runs = wanted.remove(&cell).unwrap_or_default();
        let (key, revision) = match stored.remove(&cell) {
            Some((_, current)) if current == runs => continue,
            Some((row, _)) => (row.key, Some(row.revision)),
            None if runs.is_empty() => continue,
            None => (persistence::key(PREFIX, &cell), None),
        };
        tx.put(
            &key,
            revision,
            &persistence::doc(SCHEMA, &LiveRuns { cell, runs })?,
        )?;
    }
    Ok(())
}
