use rx_domain::types::*;
use rx_ports::{Document, OwnershipFailure, Repository, StoreError};
use rx_storage::{ExclusiveFileLock, SqliteRepository};
fn refused<T>(result: Result<T, StoreError>) {
    assert!(
        matches!(result, Err(StoreError::Ownership(e)) if e.kind == OwnershipFailure::Contended)
    );
}
#[test]
fn live_writer_failed_contenders_and_rollback_preserve_exclusion_then_close_reopens_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let mut owner = SqliteRepository::open(&path).unwrap();
    for _ in 0..3 {
        refused(SqliteRepository::open(&path));
    }
    let key = Name::new("probe/key").unwrap();
    let doc = Document {
        schema: Name::new("probe.value.v1").unwrap(),
        value: serde_json::json!({"value":7}),
    };
    owner
        .transact(|tx| {
            tx.put(&key, None, &doc)?;
            refused(SqliteRepository::open(&path));
            Ok(())
        })
        .unwrap();
    let error = owner
        .transact::<()>(|tx| {
            tx.put(&Name::new("probe/rollback").unwrap(), None, &doc)?;
            Err(StoreError::Unavailable("injected rollback".into()))
        })
        .unwrap_err();
    assert!(matches!(error, StoreError::Unavailable(_)));
    refused(SqliteRepository::open(&path));
    owner.close().unwrap();
    let mut next = SqliteRepository::open(&path).unwrap();
    refused(SqliteRepository::open(&path));
    let rows = next.snapshot().unwrap().1;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, key);
    assert_eq!(rows[0].document, doc);
    drop(next);
    assert!(SqliteRepository::open(&path).is_ok());
}
#[test]
fn unwinding_transaction_does_not_release_live_owner() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let mut owner = SqliteRepository::open(&path).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), StoreError> = owner.transact(|_| panic!("injected callback unwind"));
    }));
    assert!(result.is_err());
    refused(SqliteRepository::open(&path));
    owner.check_integrity().unwrap();
    drop(owner);
    assert!(SqliteRepository::open(&path).is_ok());
}
#[test]
fn failed_initialization_releases_its_own_lock_but_failed_acquisition_never_releases_another() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("future.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 999)
        .unwrap();
    assert!(
        matches!(SqliteRepository::open(&path),Err(StoreError::Unavailable(e)) if e.contains("downgrade refused"))
    );
    let lock = ExclusiveFileLock::acquire(path.with_extension("writer.lock")).unwrap();
    assert!(
        matches!(ExclusiveFileLock::acquire(path.with_extension("writer.lock")),Err(e) if e.kind==OwnershipFailure::Contended)
    );
    assert!(
        matches!(SqliteRepository::open(&path),Err(StoreError::Ownership(e)) if e.kind==OwnershipFailure::Contended)
    );
    lock.close().unwrap();
    assert!(ExclusiveFileLock::acquire(path.with_extension("writer.lock")).is_ok());
}
