//! Offline store restore: generation rotation and its durable record. See crate::store_restore.
use super::*;
use crate::store_restore::*;
const RECORD: &str = "rx.internal.store-restore.v1";
const CURRENT: &str = "rx.internal.store-restore-ref.v1";

/// Local maintenance only, on a repository no runtime has open. Rotates the store generation
/// of the restored cut and records the restore in the same transaction. It changes no cell,
/// registration, grant or work: the next runtime start still applies its restart
/// invalidation, and every Host must be re-pinned and re-admitted.
pub fn restore_store<R: Repository>(
    repository: &mut R,
    installation_id: &Id,
    backup_sha256: Digest,
    now: TimePoint,
) -> Result<StoreRestore> {
    repository.transact(|tx| {
        let record = tx
            .get(&name("installation/current"))?
            .ok_or_else(|| StoreError::Integrity("restored store has no installation".into()))?;
        let mut installation: Installation = decode(&record, "rx.internal.installation.v1")?;
        if &installation.id != installation_id {
            return reject(Reject::InvalidInput);
        }
        let previous_generation = installation.store_generation.clone();
        installation.store_generation = id();
        tx.put(
            &name("installation/current"),
            Some(record.revision),
            &doc("rx.internal.installation.v1", &installation)?,
        )?;
        let restore = StoreRestore {
            schema: name(SCHEMA),
            id: id(),
            installation: installation.id.clone(),
            previous_generation,
            generation: installation.store_generation.clone(),
            previous_runtime_boot: installation.runtime_boot.clone(),
            backup_sha256,
            restored_at: now,
        };
        save(tx, "storerestore", &restore.id, None, RECORD, &restore)?;
        let pointer = name("storerestore/current");
        let old = tx.get(&pointer)?;
        tx.put(
            &pointer,
            old.map(|r| r.revision),
            &doc(CURRENT, &restore.id)?,
        )?;
        event(tx, "rx.event.store-restored.v1", &restore)?;
        Ok(restore)
    })
}

/// The most recent restore of this store, if any.
pub fn last_store_restore<R: Repository>(repository: &mut R) -> Result<Option<StoreRestore>> {
    repository.transact(|tx| {
        let Some(row) = tx.get(&name("storerestore/current"))? else {
            return Ok(None);
        };
        let id: Id = decode(&row, CURRENT)?;
        let (_, restore): (_, StoreRestore) = load(tx, "storerestore", &id, RECORD)?;
        if restore.id != id {
            return Err(StoreError::Integrity(
                "store restore identity differs".into(),
            ));
        }
        Ok(Some(restore))
    })
}
