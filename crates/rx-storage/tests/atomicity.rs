use rx_domain::types::*;
use rx_ports::*;
use rx_storage::SqliteRepository;
use serde_json::json;

fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn doc(n: u64) -> Document {
    Document {
        schema: name("test/entity.v1"),
        value: json!({"value": n.to_string()}),
    }
}
fn open() -> (tempfile::TempDir, SqliteRepository) {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteRepository::open(temp.path().join("platform.db")).unwrap();
    (temp, store)
}
fn scope() -> RequestScope {
    RequestScope {
        installation_id: id(),
        client_namespace: name("operator/alice"),
        method: name("Cell.StartRun"),
        key: id().to_string(),
    }
}

#[test]
fn one_transaction_commits_state_key_event_and_outbox() {
    let (_temp, mut store) = open();
    let key = name("run/example");
    let request = scope();
    let outbox = id();
    store
        .transact(|tx| {
            tx.put(&key, None, &doc(1))?;
            tx.remember(
                &request,
                &SavedRequest {
                    fingerprint: Digest::from_bytes([1; 32]),
                    result: doc(1),
                },
            )?;
            tx.append(&id(), &doc(1))?;
            tx.enqueue(&outbox, &doc(1))?;
            Ok(())
        })
        .unwrap();
    let (through, records) = store.snapshot().unwrap();
    assert_eq!(through, Counter(1));
    assert_eq!(records.len(), 1);
    assert!(store.transact(|tx| tx.lookup(&request)).unwrap().is_some());
    store
        .transact(|tx| tx.transition_outbox(&outbox, OutboxState::New, OutboxState::EmitEntered))
        .unwrap();
}

#[test]
fn failure_at_each_commit_component_rolls_everything_back() {
    for fail_after in 1..=4 {
        let (_temp, mut store) = open();
        let key = name("run/example");
        let request = scope();
        let outbox = id();
        let result = store.transact(|tx| {
            tx.put(&key, None, &doc(1))?;
            if fail_after == 1 {
                return Err(StoreError::Unavailable("injected after entity".into()));
            }
            tx.remember(
                &request,
                &SavedRequest {
                    fingerprint: Digest::from_bytes([1; 32]),
                    result: doc(1),
                },
            )?;
            if fail_after == 2 {
                return Err(StoreError::Unavailable("injected after key".into()));
            }
            tx.append(&id(), &doc(1))?;
            if fail_after == 3 {
                return Err(StoreError::Unavailable("injected after event".into()));
            }
            tx.enqueue(&outbox, &doc(1))?;
            Err::<(), _>(StoreError::Unavailable("injected after outbox".into()))
        });
        assert!(result.is_err());
        assert_eq!(store.snapshot().unwrap(), (Counter(0), vec![]));
        assert!(store.transact(|tx| tx.lookup(&request)).unwrap().is_none());
        assert!(
            store
                .transact(|tx| tx.transition_outbox(
                    &outbox,
                    OutboxState::New,
                    OutboxState::EmitEntered
                ))
                .is_err()
        );
    }
}

#[test]
fn stale_revision_does_not_partially_record_an_event() {
    let (_temp, mut store) = open();
    let key = name("cell/a");
    store.transact(|tx| tx.put(&key, None, &doc(1))).unwrap();
    let result = store.transact(|tx| {
        tx.append(&id(), &doc(2))?;
        tx.put(&key, Some(Counter(2)), &doc(2))
    });
    assert!(matches!(result, Err(StoreError::RevisionConflict(_))));
    assert_eq!(store.snapshot().unwrap().0, Counter(0));
    assert_eq!(
        store.transact(|tx| tx.get(&key)).unwrap().unwrap().document,
        doc(1)
    );
}

#[test]
fn request_identity_survives_reopening_and_namespace_isolated() {
    let (temp, mut store) = open();
    let original = scope();
    let saved = SavedRequest {
        fingerprint: Digest::from_bytes([1; 32]),
        result: doc(1),
    };
    store.transact(|tx| tx.remember(&original, &saved)).unwrap();
    drop(store);
    let mut store = SqliteRepository::open(temp.path().join("platform.db")).unwrap();
    assert_eq!(
        store.transact(|tx| tx.lookup(&original)).unwrap(),
        Some(saved.clone())
    );
    store.transact(|tx| tx.remember(&original, &saved)).unwrap();
    let changed = SavedRequest {
        fingerprint: Digest::from_bytes([2; 32]),
        result: doc(1),
    };
    assert!(matches!(
        store.transact(|tx| tx.remember(&original, &changed)),
        Err(StoreError::KeyConflict)
    ));
    let mut other = original.clone();
    other.client_namespace = name("operator/bob");
    assert!(store.transact(|tx| tx.lookup(&other)).unwrap().is_none());
}

#[test]
fn voided_outbox_cannot_be_resurrected_or_sent() {
    let (_temp, mut store) = open();
    let message = id();
    store.transact(|tx| tx.enqueue(&message, &doc(1))).unwrap();
    store
        .transact(|tx| tx.transition_outbox(&message, OutboxState::New, OutboxState::Voided))
        .unwrap();
    store.transact(|tx| tx.enqueue(&message, &doc(1))).unwrap();
    assert!(matches!(
        store.transact(|tx| tx.transition_outbox(
            &message,
            OutboxState::New,
            OutboxState::EmitEntered
        )),
        Err(StoreError::OutboxConflict)
    ));
    assert!(
        store
            .transact(|tx| tx.transition_outbox(&message, OutboxState::Voided, OutboxState::New))
            .is_err()
    );
}

#[test]
fn snapshot_is_a_cut_and_after_is_exclusive() {
    let (_temp, mut store) = open();
    store
        .transact(|tx| {
            tx.put(&name("cell/a"), None, &doc(1))?;
            tx.append(&id(), &doc(1))
        })
        .unwrap();
    let (cut, snapshot) = store.snapshot().unwrap();
    store
        .transact(|tx| {
            tx.put(&name("cell/a"), Some(Counter(1)), &doc(2))?;
            tx.append(&id(), &doc(2))
        })
        .unwrap();
    let events = store.events_after(cut, 128).unwrap();
    assert_eq!(snapshot[0].document, doc(1));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, Counter(2));
    assert!(store.events_after(Counter(2), 128).unwrap().is_empty());
}

#[test]
fn second_runtime_cannot_open_owned_database() {
    let (temp, store) = open();
    assert!(SqliteRepository::open(temp.path().join("platform.db")).is_err());
    drop(store);
    assert!(SqliteRepository::open(temp.path().join("platform.db")).is_ok());
}

#[test]
fn online_backup_preserves_committed_state_without_copying_live_db_file() {
    let (temp, mut store) = open();
    store
        .transact(|tx| {
            tx.put(&name("cell/a"), None, &doc(1))?;
            tx.append(&id(), &doc(1))
        })
        .unwrap();
    let backup = temp.path().join("snapshot.db");
    store.backup(&backup).unwrap();
    assert!(store.backup(&backup).is_err());
    let mut copy = SqliteRepository::open(&backup).unwrap();
    copy.check_integrity().unwrap();
    assert_eq!(copy.snapshot().unwrap(), store.snapshot().unwrap());
    // Restoring this snapshot to an operational installation still requires generation/Host reconciliation.
}

#[test]
fn schema_upgrade_preserves_emitted_work_and_delivered_records_never_resurrect() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("upgrade.db");
    let message = id();
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/0001.sql"))
        .unwrap();
    let document = doc(1);
    let bytes = rx_domain::canonical::bytes(&document).unwrap();
    old.execute(
        "INSERT INTO outbox(id,state,document) VALUES(?1,'EMIT_ENTERED',?2)",
        rusqlite::params![message.as_str(), bytes],
    )
    .unwrap();
    drop(old);
    let mut store = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        store.pending_outbox(128).unwrap()[0].state,
        OutboxState::EmitEntered
    );
    store
        .transact(|tx| {
            tx.transition_outbox(&message, OutboxState::EmitEntered, OutboxState::Delivered)
        })
        .unwrap();
    store
        .transact(|tx| tx.enqueue(&message, &document))
        .unwrap();
    assert!(store.pending_outbox(128).unwrap().is_empty());
    drop(store);
    let mut store = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        store
            .transact(|tx| tx.outbox(&message))
            .unwrap()
            .unwrap()
            .state,
        OutboxState::Delivered
    );
}

#[test]
fn control_projection_and_sequence_roll_back_together_and_ignore_audit_appends() {
    let (_temp, mut store) = open();
    let entity_key = name("cell/test");
    store
        .transact(|tx| {
            let entity = tx.put(&entity_key, None, &doc(1))?;
            assert_eq!(tx.append_control(&id(), &entity, &doc(1))?, Counter(1));
            Ok(())
        })
        .unwrap();
    let failed = store.transact(|tx| {
        let mut entity = tx.put(&entity_key, Some(Counter(1)), &doc(2))?;
        entity.revision = Counter(0); // Fail projection constraint after the control event INSERT.
        tx.append_control(&id(), &entity, &doc(2))?;
        Ok(())
    });
    assert!(failed.is_err());
    for _ in 0..3 {
        store.transact(|tx| tx.append(&id(), &doc(9))).unwrap();
    }
    let (head, projection) = store.control_snapshot().unwrap();
    assert_eq!(head, Counter(1));
    assert_eq!(projection[0].document, doc(1));
    assert_eq!(store.snapshot().unwrap().1[0].document, doc(1));
    assert_eq!(store.journal_head().unwrap(), Counter(3));
    store
        .transact(|tx| {
            let entity = tx.put(&entity_key, Some(Counter(1)), &doc(2))?;
            tx.append_control(&id(), &entity, &doc(2))
        })
        .unwrap();
    let after = store.control_events_after(Counter(1), 128).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].seq, Counter(2));
}

#[test]
fn pending_outbox_pages_are_exclusive_and_do_not_starve_later_ids() {
    let (_temp, mut store) = open();
    let mut ids = vec![];
    for _ in 0..19 {
        let id = id();
        store.transact(|tx| tx.enqueue(&id, &doc(1))).unwrap();
        ids.push(id);
    }
    ids.sort();
    let mut seen = vec![];
    let mut after = None;
    loop {
        let page = store.pending_outbox_after(after.as_ref(), 8).unwrap();
        if page.is_empty() {
            break;
        }
        after = page.last().map(|row| row.id.clone());
        seen.extend(page.into_iter().map(|row| row.id));
    }
    assert_eq!(seen, ids);
}

#[test]
fn metadata_compatibility_upgrade_preserves_record_bytes_and_rejects_future_stores() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("metadata-upgrade.db");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/0001.sql"))
        .unwrap();
    old.execute_batch(include_str!("../migrations/0002.sql"))
        .unwrap();
    old.execute_batch(include_str!("../migrations/0003.sql"))
        .unwrap();
    let document = Document {
        schema: name("rx.test.legacy.v1"),
        value: json!({"mode":null,"blocks":[{"id":id()}]}),
    };
    let bytes = rx_domain::canonical::bytes(&document).unwrap();
    old.execute(
        "INSERT INTO entities(key,revision,document) VALUES('legacy/context',1,?1)",
        rusqlite::params![&bytes],
    )
    .unwrap();
    drop(old);
    let mut store = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        store
            .transact(|tx| tx.get(&name("legacy/context")))
            .unwrap()
            .unwrap()
            .document,
        document
    );
    let backup = folder.path().join("metadata-backup.db");
    store.backup(&backup).unwrap();
    drop(store);
    for file in [&path, &backup] {
        let connection = rusqlite::Connection::open(file).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 6);
        let saved: Vec<u8> = connection
            .query_row(
                "SELECT document FROM entities WHERE key='legacy/context'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(saved, bytes);
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 7).unwrap();
    drop(connection);
    assert!(
        matches!(SqliteRepository::open(&path), Err(StoreError::Unavailable(message)) if message.contains("downgrade refused"))
    );
}
