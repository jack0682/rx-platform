//! Atomic persistence boundary used by application command handlers.
//! No SQLite, network transport, device implementation or asynchronous callback.
use rx_domain::types::{Counter, Digest, Id, Name};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Local ownership failures; not persisted or serialized as a wire contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipFailure {
    Contended,
    Acquire,
    ForeignProcess,
    ConnectionClose,
    Release,
}
#[derive(Debug, thiserror::Error)]
#[error("exclusive ownership {kind:?}: {detail}")]
pub struct OwnershipError {
    pub kind: OwnershipFailure,
    pub detail: String,
}
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error("application policy rejected command: {0}")]
    Rejected(rx_domain::fault::Rejection),
    #[error("store unavailable: {0}")]
    Unavailable(String),
    #[error("store integrity failure: {0}")]
    Integrity(String),
    #[error("revision conflict: {0}")]
    RevisionConflict(String),
    #[error("idempotency conflict")]
    KeyConflict,
    #[error("outbox transition conflict")]
    OutboxConflict,
    #[error("invalid document: {0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema: Name,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub key: Name,
    pub revision: Counter,
    pub document: Document,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredEvent {
    pub seq: Counter,
    pub event_id: Id,
    pub document: Document,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestScope {
    pub installation_id: Id,
    pub client_namespace: Name,
    pub method: Name,
    pub key: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SavedRequest {
    pub fingerprint: Digest,
    pub result: Document,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OutboxState {
    New,
    EmitEntered,
    Voided,
    Delivered,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxRecord {
    pub id: Id,
    pub state: OutboxState,
    pub document: Document,
}

/// All methods participate in the caller's single atomic transaction.
/// Implementations must not silently retry the callback: it can allocate command identities.
pub trait Transaction {
    /// Insert immutable source data without treating its schema as target control authority.
    fn insert_archive(&mut self, _key: &Name, _document: &Document) -> Result<Record> {
        Err(StoreError::Unavailable(
            "opaque archive insertion unsupported".into(),
        ))
    }
    fn scan_page(
        &mut self,
        _prefix: &str,
        _after: Option<&Name>,
        _limit: usize,
    ) -> Result<Vec<Record>> {
        Err(StoreError::Unavailable(
            "bounded key scan unsupported".into(),
        ))
    }
    /// Opt-in reader barrier for staged component intake. Unsupported adapters must refuse.
    fn require_component_intake_reader(&mut self) -> Result<()> {
        Err(StoreError::Unavailable(
            "component intake reader barrier unsupported".into(),
        ))
    }
    /// Import a verified historical revision only into an absent target key.
    fn insert_revision(
        &mut self,
        _key: &Name,
        _revision: Counter,
        _document: &Document,
    ) -> Result<Record> {
        Err(StoreError::Unavailable(
            "historical revision insertion unsupported".into(),
        ))
    }
    /// Append one committed control-state change and replace its current projection atomically.
    fn append_control(
        &mut self,
        event_id: &Id,
        entity: &Record,
        event: &Document,
    ) -> Result<Counter>;
    fn control_head(&mut self) -> Result<Counter>;
    fn outbox(&mut self, id: &Id) -> Result<Option<OutboxRecord>>;
    fn scan(&mut self, prefix: &str) -> Result<Vec<Record>>;
    fn get(&mut self, key: &Name) -> Result<Option<Record>>;
    fn put(&mut self, key: &Name, expected: Option<Counter>, document: &Document)
    -> Result<Record>;
    fn lookup(&mut self, scope: &RequestScope) -> Result<Option<SavedRequest>>;
    fn remember(&mut self, scope: &RequestScope, request: &SavedRequest) -> Result<()>;
    fn append(&mut self, event_id: &Id, document: &Document) -> Result<Counter>;
    fn enqueue(&mut self, id: &Id, document: &Document) -> Result<()>;
    fn transition_outbox(
        &mut self,
        id: &Id,
        expected: OutboxState,
        next: OutboxState,
    ) -> Result<()>;
}

pub trait Repository {
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<OutboxRecord>>;
    fn control_events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>>;
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)>;
    /// Highest committed local event sequence; does not materialize entity snapshots.
    fn journal_head(&mut self) -> Result<Counter>;
    fn pending_outbox(&mut self, limit: usize) -> Result<Vec<OutboxRecord>>;
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> Result<T>,
    ) -> Result<T>;
    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)>;
    fn events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>>;
}

/// Trusted persistence adapter capability, never a caller-supplied seal claim.
pub trait SealedRepository: Repository {
    fn sealed_namespaces(&self) -> Result<Vec<Name>>;
}
