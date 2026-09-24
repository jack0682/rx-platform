//! Local SQLite adapter. One process owns the lock and one worker owns this connection.
//! No network or native-device calls may be made from a transaction callback.
mod ownership;
pub use ownership::ExclusiveFileLock;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rx_domain::{canonical, types::*};
use rx_ports::*;
use serde::de::DeserializeOwned;
use std::{path::Path, time::Duration};

pub struct SqliteRepository {
    // This order is deliberate. Explicit close also confirms Connection::close
    // before release; close failure keeps both resources until process exit.
    connection: Option<Connection>,
    ownership: Option<ExclusiveFileLock>,
}
fn unavailable(e: impl std::fmt::Display) -> StoreError {
    StoreError::Unavailable(e.to_string())
}
fn integrity(e: impl std::fmt::Display) -> StoreError {
    StoreError::Integrity(e.to_string())
}
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = canonical::bytes(value).map_err(integrity)?;
    if bytes.len() > canonical::MAX_MESSAGE_BYTES {
        return Err(StoreError::Invalid("document exceeds 1 MiB".into()));
    }
    Ok(bytes)
}
fn decode<T: DeserializeOwned>(value: Vec<u8>) -> Result<T> {
    canonical::decode_json(&value).map_err(integrity)
}
fn counter(n: i64) -> Result<Counter> {
    u64::try_from(n)
        .map(Counter)
        .map_err(|_| StoreError::Integrity("negative stored counter".into()))
}
fn signed(c: Counter) -> Result<i64> {
    i64::try_from(c.0).map_err(|_| StoreError::Invalid("SQLite revision range exhausted".into()))
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| StoreError::Invalid("database needs parent path".into()))?;
        std::fs::create_dir_all(parent).map_err(unavailable)?;
        let ownership = ExclusiveFileLock::acquire(path.with_extension("writer.lock"))?;
        // Version pin alone is insufficient; check the SQLite library actually linked.
        if rusqlite::version_number() < 3_051_003 {
            return Err(StoreError::Unavailable(
                "SQLite with WAL-reset fix (3.51.3+) required".into(),
            ));
        }
        let connection = Connection::open(path).map_err(unavailable)?;
        let repository = Self {
            connection: Some(connection),
            ownership: Some(ownership),
        };
        repository.initialize()?;
        Ok(repository)
    }
    fn initialize(&self) -> Result<()> {
        let connection = self.connection()?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(unavailable)?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(StoreError::Unavailable("local WAL unavailable".into()));
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(unavailable)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(unavailable)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(unavailable)?;
        if version > 6 {
            return Err(StoreError::Unavailable(
                "newer store schema: downgrade refused".into(),
            ));
        }
        if version == 0 {
            connection
                .execute_batch(include_str!("../migrations/0001.sql"))
                .map_err(unavailable)?;
        }
        if version < 2 {
            connection
                .execute_batch(include_str!("../migrations/0002.sql"))
                .map_err(unavailable)?;
        }
        if version < 3 {
            connection
                .execute_batch(include_str!("../migrations/0003.sql"))
                .map_err(unavailable)?;
        }
        if version < 4 {
            connection
                .execute_batch(include_str!("../migrations/0004.sql"))
                .map_err(unavailable)?;
        }
        if version < 5 {
            connection
                .execute_batch(include_str!("../migrations/0005.sql"))
                .map_err(unavailable)?;
        }
        if version < 6 {
            connection
                .execute_batch(include_str!("../migrations/0006.sql"))
                .map_err(unavailable)?;
        }
        Ok(())
    }
    fn connection(&self) -> Result<&Connection> {
        self.ownership
            .as_ref()
            .expect("open repository")
            .check_process()?;
        Ok(self.connection.as_ref().expect("open connection"))
    }
    fn connection_mut(&mut self) -> Result<&mut Connection> {
        self.ownership
            .as_ref()
            .expect("open repository")
            .check_process()?;
        Ok(self.connection.as_mut().expect("open connection"))
    }
    /// Consume the repository; never release its ownership before SQLite confirms
    /// close. Failure deliberately retains resources until this process exits.
    pub fn close(mut self) -> Result<()> {
        self.close_inner()
    }
    fn close_inner(&mut self) -> Result<()> {
        let Some(owner) = self.ownership.take() else {
            return Ok(());
        };
        if let Err(error) = owner.check_process() {
            // Do not call SQLite destructors on a fork-inherited connection.
            if let Some(connection) = self.connection.take() {
                std::mem::forget(connection);
            }
            drop(owner); // creator check prevents inherited LOCK_UN
            return Err(error.into());
        }
        // A dependency panic during close must not run an unlocking guard's
        // destructor before connection closure has been confirmed.
        let owner = std::mem::ManuallyDrop::new(owner);
        if let Some(connection) = self.connection.take()
            && let Err((connection, error)) = connection.close()
        {
            std::mem::forget(connection);
            std::mem::ManuallyDrop::into_inner(owner).retain_until_process_exit();
            return Err(OwnershipError {
                    kind: OwnershipFailure::ConnectionClose,
                    detail: format!(
                        "SQLite close unconfirmed; this process retains connection and lock until exit; inherited copies may retain ownership longer: {error}"
                    ),
                }
                .into());
        }
        std::mem::ManuallyDrop::into_inner(owner).close()?;
        Ok(())
    }
    pub fn sqlite_version(&self) -> &'static str {
        rusqlite::version()
    }
    pub fn backup(&self, path: impl AsRef<Path>) -> Result<()> {
        self.connection()?; // reject inherited access before path observation
        if path.as_ref().exists() {
            return Err(StoreError::Invalid("backup destination must be new".into()));
        }
        self.connection()?
            .backup("main", path, None)
            .map_err(unavailable)
    }
    pub fn check_integrity(&self) -> Result<()> {
        let result: String = self
            .connection()?
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(unavailable)?;
        if result != "ok" {
            return Err(StoreError::Integrity(result));
        }
        Ok(())
    }
}

impl Drop for SqliteRepository {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

impl Repository for SqliteRepository {
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<OutboxRecord>> {
        self.connection()?;
        if limit == 0 || limit > 128 {
            return Err(StoreError::Invalid("outbox batch must be 1..128".into()));
        }
        let mut query=self.connection()?.prepare("SELECT id,state,document FROM outbox WHERE state IN ('NEW','EMIT_ENTERED') AND (?1 IS NULL OR id>?1) ORDER BY id LIMIT ?2").map_err(unavailable)?;
        let rows = query
            .query_map(params![after.map(Id::as_str), limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(unavailable)?;
        rows.map(|r| {
            let (id, state, document) = r.map_err(unavailable)?;
            Ok(OutboxRecord {
                id: Id::new(id).map_err(integrity)?,
                state: outbox_state(&state)?,
                document: decode(document)?,
            })
        })
        .collect()
    }
    fn control_events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>> {
        read_events(self.connection()?, "control_events", after, limit)
    }
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        let tx = self.connection_mut()?.transaction().map_err(unavailable)?;
        let seq = read_head(&tx, "control_events")?;
        let records = read_entities(&tx, "control_entities")?;
        tx.commit().map_err(unavailable)?;
        Ok((seq, records))
    }
    fn journal_head(&mut self) -> Result<Counter> {
        let seq: i64 = self
            .connection()?
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |row| {
                row.get(0)
            })
            .map_err(unavailable)?;
        counter(seq)
    }
    fn pending_outbox(&mut self, limit: usize) -> Result<Vec<OutboxRecord>> {
        self.connection()?;
        if limit == 0 || limit > 128 {
            return Err(StoreError::Invalid("outbox batch must be 1..128".into()));
        }
        let mut query=self.connection()?.prepare("SELECT id,state,document FROM outbox WHERE state IN ('NEW','EMIT_ENTERED') ORDER BY id LIMIT ?1").map_err(unavailable)?;
        let rows = query
            .query_map([limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(unavailable)?;
        rows.map(|r| {
            let (id, state, document) = r.map_err(unavailable)?;
            Ok(OutboxRecord {
                id: Id::new(id).map_err(integrity)?,
                state: outbox_state(&state)?,
                document: decode(document)?,
            })
        })
        .collect()
    }
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn rx_ports::Transaction) -> Result<T>,
    ) -> Result<T> {
        let creator = self.ownership.as_ref().expect("open repository").creator();
        let transaction = TransactionScope {
            transaction: Some(
                self.connection_mut()?
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(unavailable)?,
            ),
            creator,
        };
        let outcome = operation(&mut SqliteTransaction {
            transaction: transaction.transaction.as_ref().expect("live transaction"),
            creator,
        });
        // The callback may fork. Refuse its inherited commit even when it
        // returns Ok, and never run SQLite rollback from the foreign process.
        ownership::check_creator(creator)?;
        let value = outcome?;
        transaction.commit()?;
        Ok(value)
    }

    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        let tx = self.connection_mut()?.transaction().map_err(unavailable)?;
        let seq: i64 = tx
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
            .map_err(unavailable)?;
        let records = {
            let mut query = tx
                .prepare("SELECT key,revision,document FROM entities ORDER BY key")
                .map_err(unavailable)?;
            let rows = query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })
                .map_err(unavailable)?;
            let mut records = Vec::new();
            for row in rows {
                let (key, revision, document) = row.map_err(unavailable)?;
                records.push(Record {
                    key: Name::new(key).map_err(integrity)?,
                    revision: counter(revision)?,
                    document: decode(document)?,
                });
            }
            records
        };
        tx.commit().map_err(unavailable)?;
        Ok((counter(seq)?, records))
    }
    fn events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>> {
        self.connection()?;
        if limit == 0 || limit > 128 {
            return Err(StoreError::Invalid("event batch must be 1..128".into()));
        }
        let mut query = self
            .connection()?
            .prepare("SELECT seq,event_id,document FROM events WHERE seq>?1 ORDER BY seq LIMIT ?2")
            .map_err(unavailable)?;
        let rows = query
            .query_map(params![signed(after)?, limit as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(unavailable)?;
        let mut records = Vec::new();
        for row in rows {
            let (seq, event_id, document) = row.map_err(unavailable)?;
            records.push(StoredEvent {
                seq: counter(seq)?,
                event_id: Id::new(event_id).map_err(integrity)?,
                document: decode(document)?,
            });
        }
        Ok(records)
    }
}

struct TransactionScope<'a> {
    transaction: Option<rusqlite::Transaction<'a>>,
    creator: u32,
}
impl TransactionScope<'_> {
    fn commit(mut self) -> Result<()> {
        ownership::check_creator(self.creator)?;
        self.transaction
            .take()
            .expect("live transaction")
            .commit()
            .map_err(unavailable)
    }
}
impl Drop for TransactionScope<'_> {
    fn drop(&mut self) {
        if self.creator != std::process::id()
            && let Some(transaction) = self.transaction.take()
        {
            std::mem::forget(transaction);
        }
        // In the creator, normal SQLite rollback runs on error/unwind.
    }
}
struct SqliteTransaction<'a, 'b> {
    transaction: &'a rusqlite::Transaction<'b>,
    creator: u32,
}
impl SqliteTransaction<'_, '_> {
    fn check_process(&self) -> Result<()> {
        Ok(ownership::check_creator(self.creator)?)
    }
}
impl rx_ports::Transaction for SqliteTransaction<'_, '_> {
    fn control_head(&mut self) -> Result<Counter> {
        self.check_process()?;
        read_head(self.transaction, "control_events")
    }
    fn append_control(
        &mut self,
        event_id: &Id,
        entity: &Record,
        event: &Document,
    ) -> Result<Counter> {
        self.check_process()?;
        let event_bytes = encode(event)?;
        self.transaction
            .execute(
                "INSERT INTO control_events(event_id,document) VALUES(?1,?2)",
                params![event_id.as_str(), event_bytes],
            )
            .map_err(unavailable)?;
        let seq = counter(self.transaction.last_insert_rowid())?;
        self.transaction.execute("INSERT INTO control_entities(key,revision,document) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision,document=excluded.document",
            params![entity.key.as_str(),signed(entity.revision)?,encode(&entity.document)?]).map_err(unavailable)?;
        Ok(seq)
    }
    fn outbox(&mut self, id: &Id) -> Result<Option<OutboxRecord>> {
        self.check_process()?;
        let row = self
            .transaction
            .query_row(
                "SELECT state,document FROM outbox WHERE id=?1",
                [id.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(unavailable)?;
        row.map(|(state, document)| {
            Ok(OutboxRecord {
                id: id.clone(),
                state: outbox_state(&state)?,
                document: decode(document)?,
            })
        })
        .transpose()
    }
    fn scan(&mut self, prefix: &str) -> Result<Vec<Record>> {
        self.check_process()?;
        if prefix.is_empty() || prefix.contains(['%', '_']) {
            return Err(StoreError::Invalid("invalid internal key prefix".into()));
        }
        let mut statement = self
            .transaction
            .prepare("SELECT key,revision,document FROM entities WHERE key LIKE ?1 ORDER BY key")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([format!("{prefix}%")], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(unavailable)?;
        rows.map(|row| {
            let (key, revision, document) = row.map_err(unavailable)?;
            Ok(Record {
                key: Name::new(key).map_err(integrity)?,
                revision: counter(revision)?,
                document: decode(document)?,
            })
        })
        .collect()
    }
    fn get(&mut self, key: &Name) -> Result<Option<Record>> {
        self.check_process()?;
        let row = self
            .transaction
            .query_row(
                "SELECT revision,document FROM entities WHERE key=?1",
                [key.as_str()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(unavailable)?;
        row.map(|(revision, document)| {
            Ok(Record {
                key: key.clone(),
                revision: counter(revision)?,
                document: decode(document)?,
            })
        })
        .transpose()
    }
    fn put(
        &mut self,
        key: &Name,
        expected: Option<Counter>,
        document: &Document,
    ) -> Result<Record> {
        self.check_process()?;
        let current = self.get(key)?;
        if current.as_ref().map(|r| r.revision) != expected {
            return Err(StoreError::RevisionConflict(key.to_string()));
        }
        let revision = expected
            .unwrap_or(Counter(0))
            .increment()
            .map_err(integrity)?;
        self.transaction.execute("INSERT INTO entities(key,revision,document) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision, document=excluded.document",
            params![key.as_str(), signed(revision)?, encode(document)?]).map_err(unavailable)?;
        Ok(Record {
            key: key.clone(),
            revision,
            document: document.clone(),
        })
    }
    fn lookup(&mut self, scope: &RequestScope) -> Result<Option<SavedRequest>> {
        self.check_process()?;
        let row = self
            .transaction
            .query_row(
                "SELECT fingerprint,result FROM requests WHERE scope=?1",
                [encode(scope)?],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(unavailable)?;
        row.map(|(digest, result)| {
            let bytes: [u8; 32] = digest
                .try_into()
                .map_err(|_| StoreError::Integrity("digest length".into()))?;
            Ok(SavedRequest {
                fingerprint: Digest::from_bytes(bytes),
                result: decode(result)?,
            })
        })
        .transpose()
    }
    fn remember(&mut self, scope: &RequestScope, request: &SavedRequest) -> Result<()> {
        self.check_process()?;
        if let Some(old) = self.lookup(scope)? {
            if old != *request {
                return Err(StoreError::KeyConflict);
            }
            return Ok(());
        }
        self.transaction
            .execute(
                "INSERT INTO requests(scope,fingerprint,result) VALUES(?1,?2,?3)",
                params![
                    encode(scope)?,
                    request.fingerprint.as_bytes().as_slice(),
                    encode(&request.result)?
                ],
            )
            .map_err(unavailable)?;
        Ok(())
    }
    fn append(&mut self, event_id: &Id, document: &Document) -> Result<Counter> {
        self.check_process()?;
        let old: Option<(i64, Vec<u8>)> = self
            .transaction
            .query_row(
                "SELECT seq,document FROM events WHERE event_id=?1",
                [event_id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(unavailable)?;
        let payload = encode(document)?;
        if let Some((seq, old)) = old {
            if old != payload {
                return Err(StoreError::KeyConflict);
            }
            return counter(seq);
        }
        self.transaction
            .execute(
                "INSERT INTO events(event_id,document) VALUES(?1,?2)",
                params![event_id.as_str(), payload],
            )
            .map_err(unavailable)?;
        Ok(Counter(self.transaction.last_insert_rowid() as u64))
    }
    fn enqueue(&mut self, id: &Id, document: &Document) -> Result<()> {
        self.check_process()?;
        let old: Option<Vec<u8>> = self
            .transaction
            .query_row(
                "SELECT document FROM outbox WHERE id=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(unavailable)?;
        let payload = encode(document)?;
        if let Some(old) = old {
            if old != payload {
                return Err(StoreError::KeyConflict);
            }
            return Ok(()); // Never resurrect a VOIDED or EMIT_ENTERED record.
        }
        self.transaction
            .execute(
                "INSERT INTO outbox(id,state,document) VALUES(?1,'NEW',?2)",
                params![id.as_str(), payload],
            )
            .map_err(unavailable)?;
        Ok(())
    }
    fn transition_outbox(
        &mut self,
        id: &Id,
        expected: OutboxState,
        next: OutboxState,
    ) -> Result<()> {
        self.check_process()?;
        let (from, to) = match (expected, next) {
            (OutboxState::New, OutboxState::EmitEntered) => ("NEW", "EMIT_ENTERED"),
            (OutboxState::New, OutboxState::Voided) => ("NEW", "VOIDED"),
            (OutboxState::EmitEntered, OutboxState::Delivered) => ("EMIT_ENTERED", "DELIVERED"),
            _ => return Err(StoreError::OutboxConflict),
        };
        let changed = self
            .transaction
            .execute(
                "UPDATE outbox SET state=?1 WHERE id=?2 AND state=?3",
                params![to, id.as_str(), from],
            )
            .map_err(unavailable)?;
        if changed != 1 {
            return Err(StoreError::OutboxConflict);
        }
        Ok(())
    }
}

fn outbox_state(s: &str) -> Result<OutboxState> {
    match s {
        "NEW" => Ok(OutboxState::New),
        "EMIT_ENTERED" => Ok(OutboxState::EmitEntered),
        "VOIDED" => Ok(OutboxState::Voided),
        "DELIVERED" => Ok(OutboxState::Delivered),
        _ => Err(StoreError::Integrity("invalid outbox state".into())),
    }
}

// Table names are private constants from adapter methods, never caller input.
fn read_head(connection: &Connection, table: &str) -> Result<Counter> {
    let seq: i64 = connection
        .query_row(
            &format!("SELECT COALESCE(MAX(seq),0) FROM {table}"),
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    counter(seq)
}
fn read_entities(connection: &Connection, table: &str) -> Result<Vec<Record>> {
    let mut query = connection
        .prepare(&format!(
            "SELECT key,revision,document FROM {table} ORDER BY key"
        ))
        .map_err(unavailable)?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(unavailable)?;
    rows.map(|row| {
        let (key, revision, document) = row.map_err(unavailable)?;
        Ok(Record {
            key: Name::new(key).map_err(integrity)?,
            revision: counter(revision)?,
            document: decode(document)?,
        })
    })
    .collect()
}
fn read_events(
    connection: &Connection,
    table: &str,
    after: Counter,
    limit: usize,
) -> Result<Vec<StoredEvent>> {
    if limit == 0 || limit > 128 {
        return Err(StoreError::Invalid("event batch must be 1..128".into()));
    }
    let mut query = connection
        .prepare(&format!(
            "SELECT seq,event_id,document FROM {table} WHERE seq>?1 ORDER BY seq LIMIT ?2"
        ))
        .map_err(unavailable)?;
    let rows = query
        .query_map(params![signed(after)?, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(unavailable)?;
    rows.map(|row| {
        let (seq, id, document) = row.map_err(unavailable)?;
        Ok(StoredEvent {
            seq: counter(seq)?,
            event_id: Id::new(id).map_err(integrity)?,
            document: decode(document)?,
        })
    })
    .collect()
}

#[cfg(test)]
mod ownership_close_tests {
    use super::*;
    #[test]
    fn sqlite_close_failure_retains_ownership_until_process_exit() {
        const CHILD: &str = "RX_STORAGE_CLOSE_FAILURE_PROBE";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = std::path::PathBuf::from(root);
            let store = SqliteRepository::open(root.join("state.db")).unwrap();
            // A real unfinalized statement makes sqlite3_close fail. This private
            // test does not add a Connection/statement export to the public API.
            let statement = store.connection().unwrap().prepare("SELECT 1").unwrap();
            std::mem::forget(statement);
            let error = store.close().unwrap_err();
            assert!(
                matches!(&error, StoreError::Ownership(e) if e.kind == OwnershipFailure::ConnectionClose)
            );
            std::fs::write(root.join("refused"), error.to_string()).unwrap();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !root.join("finish").exists() {
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            return;
        }
        let root = tempfile::tempdir().unwrap();
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let log = std::fs::File::create(root.path().join("child.log")).unwrap();
        let mut child = Child(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ownership_close_tests::sqlite_close_failure_retains_ownership_until_process_exit", "--nocapture"])
            .env(CHILD, root.path())
            .stdout(std::process::Stdio::from(log.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(log)).spawn().unwrap());
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.path().join("refused").exists() {
            assert!(child.0.try_wait().unwrap().is_none());
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            matches!(SqliteRepository::open(root.path().join("state.db")), Err(StoreError::Ownership(e)) if e.kind==OwnershipFailure::Contended)
        );
        std::fs::write(root.path().join("finish"), "finish").unwrap();
        assert!(child.0.wait().unwrap().success());
        assert!(SqliteRepository::open(root.path().join("state.db")).is_ok());
        println!(
            "SQLITE_CLOSE_UNCONFIRMED: no unlock; contender refused until holder process exits"
        );
    }
}
