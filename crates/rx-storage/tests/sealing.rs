use rx_domain::types::*;
use rx_ports::*;
use rx_storage::SqliteRepository;
use serde_json::json;
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn document(value: u64) -> Document {
    Document {
        schema: n("test/value.v1"),
        value: json!({"value":value}),
    }
}

#[test]
fn freeze_and_marker_are_atomic_and_only_selected_namespaces_are_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    let mut db = SqliteRepository::open(&path).unwrap();
    db.transact(|tx| {
        tx.put(&n("components/registration/a"), None, &document(1))?;
        Ok(())
    })
    .unwrap();
    let result: Result<()> =
        db.seal_prefixes(&[n("components/registration/"), n("freeze/")], |tx| {
            tx.put(&n("freeze/marker"), None, &document(1))?;
            Err(StoreError::Unavailable("before commit".into()))
        });
    assert!(result.is_err());
    assert!(db.sealed_prefixes().unwrap().is_empty());
    assert!(
        db.transact(|tx| tx.get(&n("freeze/marker")))
            .unwrap()
            .is_none()
    );
    db.seal_prefixes(&[n("components/registration/"), n("freeze/")], |tx| {
        tx.put(&n("freeze/marker"), None, &document(1))?;
        Ok(())
    })
    .unwrap();
    assert!(
        db.transact(|tx| tx.put(
            &n("components/registration/a"),
            Some(Counter(1)),
            &document(2)
        ))
        .is_err()
    );
    assert!(
        db.transact(|tx| tx.put(&n("components/registration/new"), None, &document(2)))
            .is_err()
    );
    assert!(
        db.transact(|tx| tx.put(&n("freeze/marker"), Some(Counter(1)), &document(2)))
            .is_err()
    );
    db.transact(|tx| {
        let row = tx.put(&n("components/execution/a/x"), None, &document(2))?;
        tx.append_control(&id(), &row, &document(2))?;
        tx.put(&n("components/registration-sibling/a"), None, &document(3))?;
        Ok(())
    })
    .unwrap();
    db.close().unwrap();
    let mut db = SqliteRepository::open(&path).unwrap();
    assert_eq!(db.sealed_prefixes().unwrap().len(), 2);
    db.seal_prefixes(&[n("components/registration/"), n("freeze/")], |tx| {
        assert!(tx.get(&n("freeze/marker"))?.is_some());
        Ok(())
    })
    .unwrap();
    db.close().unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        7
    );
    for sql in [
        "DELETE FROM entities WHERE key='components/registration/a'",
        "UPDATE entities SET key='elsewhere/a' WHERE key='components/registration/a'",
        "INSERT OR REPLACE INTO entities(key,revision,document) VALUES('components/registration/a',2,X'7b7d')",
        "DELETE FROM sealed_prefixes",
        "UPDATE sealed_prefixes SET prefix='elsewhere/'",
        "INSERT INTO control_entities(key,revision,document) VALUES('components/registration/a',1,X'7b7d')",
    ] {
        let error = raw.execute_batch(sql).unwrap_err();
        assert!(error.to_string().contains("namespace"), "{sql}: {error}");
    }
}

#[test]
fn missing_guard_is_detected_and_ordinary_databases_stay_at_schema_six() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    let mut db = SqliteRepository::open(&path).unwrap();
    db.close().unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    drop(raw);
    db = SqliteRepository::open(&path).unwrap();
    db.seal_prefixes(&[n("declaration/")], |_| Ok(())).unwrap();
    db.close().unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("DROP TRIGGER entities_sealed_insert")
        .unwrap();
    drop(raw);
    assert!(SqliteRepository::open(&path).is_err());
}

#[test]
fn ddl_failure_after_staging_rolls_back_marker_schema_and_new_seals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    SqliteRepository::open(&path).unwrap().close().unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("CREATE TRIGGER control_entities_sealed_update AFTER UPDATE ON entities BEGIN SELECT 1; END").unwrap();
    drop(raw);
    let mut db = SqliteRepository::open(&path).unwrap();
    assert!(
        db.seal_prefixes(&[n("frozen/")], |tx| {
            tx.put(&n("frozen/marker"), None, &document(1))?;
            Ok(())
        })
        .is_err()
    );
    assert!(db.sealed_prefixes().unwrap().is_empty());
    assert!(
        db.transact(|tx| tx.get(&n("frozen/marker")))
            .unwrap()
            .is_none()
    );
    db.transact(|tx| tx.put(&n("frozen/positive"), None, &document(2)))
        .unwrap();
    db.close().unwrap();
    let raw = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='sealed_prefixes'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
