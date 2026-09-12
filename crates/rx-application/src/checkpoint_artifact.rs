//! Content-addressed executor checkpoint snapshots, built inside the state transaction.
use crate::{model::*, persistence::*};
use rx_domain::{canonical, types::*};
use rx_ports::*;
use sha2::{Digest as _, Sha256};

pub const SCHEMA: &str = "rx.executor-state.v1";
pub const RUN_SNAPSHOT: &str = "rx.control.run-snapshot.v1";
pub use rx_process_contract::execution::{
    ActivationSnapshot, CheckpointView, ExecutorState, RunSnapshot, SlotSnapshot,
};

pub(crate) fn capture_run(tx: &mut dyn Transaction, record: &Record) -> Result<Record> {
    let run: Run = decode(record, "rx.internal.run.v1")?;
    let mut activations = vec![];
    for entry in tx.scan("activation/")? {
        let activation: Activation = decode(&entry, "rx.internal.activation.v1")?;
        if activation.run != run.id {
            continue;
        }
        activations.push(activation_snapshot(tx, &activation, &run.cell)?);
    }
    activations.sort_by(|a, b| (&a.node, a.visit).cmp(&(&b.node, b.visit)));
    let mut process_checkpoints = vec![];
    for entry in tx.scan("checkpoint/")? {
        let checkpoint: ProcessCheckpoint = decode(&entry, "rx.internal.process-checkpoint.v1")?;
        if checkpoint.run == run.id {
            process_checkpoints.push(checkpoint);
        }
    }
    process_checkpoints.sort_by_key(|checkpoint| checkpoint.visit);
    let state = ExecutorState {
        schema: name(SCHEMA),
        run: run.clone(),
        revision: record.revision,
        activations: activations.clone(),
        process_checkpoints,
    };
    let checkpoint = store_state(tx, &state)?;
    let snapshot = RunSnapshot {
        revision: record.revision,
        run: run.clone(),
        checkpoint,
    };
    let current = key("runcheckpoint", &run.id);
    let prior = tx.get(&current)?;
    let document = doc(RUN_SNAPSHOT, &snapshot)?;
    tx.put(&current, prior.map(|row| row.revision), &document)?;
    Ok(Record {
        key: record.key.clone(),
        revision: record.revision,
        document,
    })
}
/// Store an immutable typed state artifact; this does not replace the current checkpoint.
pub(crate) fn store_state(
    tx: &mut dyn Transaction,
    state: &ExecutorState,
) -> Result<CheckpointView> {
    let bytes = canonical::bytes(state).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let reference = ArtifactRef {
        sha256: Digest::from_bytes(Sha256::digest(&bytes).into()),
        schema_id: name(SCHEMA),
        size_bytes: Counter(bytes.len() as u64),
    };
    let blob_key = key("artifact", reference.sha256);
    let blob = doc(SCHEMA, state)?;
    if let Some(previous) = tx.get(&blob_key)? {
        if previous.document != blob {
            return Err(StoreError::Integrity(
                "checkpoint content hash collision".into(),
            ));
        }
    } else {
        tx.put(&blob_key, None, &blob)?;
    }
    let owner = key("checkpointartifact", (&state.run.id, reference.sha256));
    if tx.get(&owner)?.is_none() {
        tx.put(
            &owner,
            None,
            &doc("rx.internal.checkpoint-artifact-ref.v1", &reference)?,
        )?;
    }
    Ok(CheckpointView {
        run: state.run.id.clone(),
        revision: state.revision,
        executor_schema: name(SCHEMA),
        payload: reference,
        activations: state.activations.clone(),
    })
}

pub(crate) fn read_artifact(
    tx: &mut dyn Transaction,
    run: &Id,
    reference: &ArtifactRef,
) -> Result<Vec<u8>> {
    let (_, owned): (_, ArtifactRef) = load(
        tx,
        "checkpointartifact",
        (run, reference.sha256),
        "rx.internal.checkpoint-artifact-ref.v1",
    )?;
    if owned != *reference || reference.schema_id.as_str() != SCHEMA {
        return Err(StoreError::Invalid(
            "checkpoint artifact reference mismatch".into(),
        ));
    }
    let row = tx
        .get(&key("artifact", reference.sha256))?
        .ok_or(StoreError::Integrity("checkpoint artifact missing".into()))?;
    let state: ExecutorState = decode(&row, SCHEMA)?;
    if &state.run.id != run || state.schema.as_str() != SCHEMA {
        return Err(StoreError::Integrity(
            "checkpoint artifact owner mismatch".into(),
        ));
    }
    let bytes = canonical::bytes(&state).map_err(|e| StoreError::Integrity(e.to_string()))?;
    if reference.size_bytes.0 != bytes.len() as u64
        || reference.sha256 != Digest::from_bytes(Sha256::digest(&bytes).into())
    {
        return Err(StoreError::Integrity(
            "checkpoint artifact content differs".into(),
        ));
    }
    Ok(bytes)
}

/// Validate the materialized view against its immutable content before serving it.
pub(crate) fn validate_snapshot(tx: &mut dyn Transaction, snapshot: &RunSnapshot) -> Result<()> {
    let checkpoint = &snapshot.checkpoint;
    if checkpoint.run != snapshot.run.id
        || checkpoint.revision != snapshot.revision
        || checkpoint.executor_schema.as_str() != SCHEMA
    {
        return Err(StoreError::Integrity(
            "checkpoint view identity differs".into(),
        ));
    }
    let bytes = read_artifact(tx, &snapshot.run.id, &checkpoint.payload)?;
    let state: ExecutorState =
        canonical::decode_json(&bytes).map_err(|e| StoreError::Integrity(e.to_string()))?;
    let encode = |value| canonical::bytes(value).map_err(|e| StoreError::Integrity(e.to_string()));
    if state.revision != snapshot.revision
        || encode(&state.run)? != encode(&snapshot.run)?
        || canonical::bytes(&state.activations).map_err(|e| StoreError::Integrity(e.to_string()))?
            != canonical::bytes(&checkpoint.activations)
                .map_err(|e| StoreError::Integrity(e.to_string()))?
    {
        return Err(StoreError::Integrity(
            "checkpoint view/content cut differs".into(),
        ));
    }
    Ok(())
}

pub(crate) fn activation_snapshot(
    tx: &mut dyn Transaction,
    activation: &Activation,
    cell: &Name,
) -> Result<ActivationSnapshot> {
    let mut slots = vec![];
    for (slot, operation) in &activation.slots {
        let (_, work): (_, Work) = load(tx, "work", operation, "rx.internal.work.v1")?;
        if work.run != activation.run
            || work.activation != activation.id
            || work.slot != *slot
            || work.operation.id() != operation
            || work.part != activation.part
            || &work.cell != cell
        {
            return Err(StoreError::Integrity(
                "checkpoint slot/work mismatch".into(),
            ));
        }
        slots.push(SlotSnapshot {
            slot: slot.clone(),
            operation: operation.clone(),
            intent_digest: work
                .intent
                .digest()
                .map_err(|e| StoreError::Invalid(e.to_string()))?,
        });
    }
    Ok(ActivationSnapshot {
        id: activation.id.clone(),
        node: activation.node.clone(),
        visit: activation.visit,
        slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_storage::SqliteRepository;
    #[test]
    fn rejects_a_valid_older_artifact_substituted_into_a_newer_run_view() {
        let directory = tempfile::tempdir().unwrap();
        let mut repository = SqliteRepository::open(directory.path().join("p.db")).unwrap();
        let mut run = Run {
            id: id(),
            cell: name("cell/a"),
            recipe_digest: Digest::from_bytes([1; 32]),
            envelope_digest: Digest::from_bytes([2; 32]),
            purpose: None,
            state: RunState::Prepared,
            budget: None,
            executor_session: None,
            mandate: None,
            part_ids: vec![],
            pending_attempt: None,
        };
        let older: RunSnapshot = repository
            .transact(|tx| {
                let row = tx.put(
                    &key("run", &run.id),
                    None,
                    &doc("rx.internal.run.v1", &run)?,
                )?;
                decode(&capture_run(tx, &row)?, RUN_SNAPSHOT)
            })
            .unwrap();
        run.state = RunState::Paused;
        let newer: RunSnapshot = repository
            .transact(|tx| {
                let row = tx.put(
                    &key("run", &run.id),
                    Some(Counter(1)),
                    &doc("rx.internal.run.v1", &run)?,
                )?;
                decode(&capture_run(tx, &row)?, RUN_SNAPSHOT)
            })
            .unwrap();
        repository
            .transact(|tx| {
                validate_snapshot(tx, &older)?;
                validate_snapshot(tx, &newer)
            })
            .unwrap();
        let mut mismatched = newer.clone();
        mismatched.checkpoint.payload = older.checkpoint.payload.clone();
        assert!(matches!(
            repository.transact(|tx| validate_snapshot(tx, &mismatched)),
            Err(StoreError::Integrity(_))
        ));
        repository
            .transact(|tx| {
                let blob_key = key("artifact", newer.checkpoint.payload.sha256);
                let mut blob = tx.get(&blob_key)?.unwrap();
                blob.document.value["run"]["state"] = serde_json::json!("COMPLETED");
                tx.put(&blob_key, Some(blob.revision), &blob.document)?;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            repository.transact(|tx| read_artifact(tx, &run.id, &newer.checkpoint.payload)),
            Err(StoreError::Integrity(_))
        ));
        repository
            .transact(|tx| validate_snapshot(tx, &older))
            .unwrap();
    }
}
