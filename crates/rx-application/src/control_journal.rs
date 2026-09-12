//! Atomic, typed control-state journal. Internal audit events use a separate sequence space.
//! Wire projection is a boundary concern; stored entities are immutable snapshots at this cut.
use crate::{model::*, persistence::*};
use rx_domain::{canonical, types::*};
use rx_ports::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const VIEW_ID: &str = "site-cell-control-v1";
pub const CHANGE_SCHEMA: &str = "rx.control.change.v1";
pub const ADMISSION_SCHEMA: &str = "rx.internal.admission-receipt.v1";
pub fn journal_id(installation: &Installation) -> Result<Id> {
    let digest = canonical::digest(
        "RX-CONTROL-JOURNAL-ID-v1",
        &(&installation.id, &installation.store_generation, VIEW_ID),
    )
    .map_err(|e| StoreError::Invalid(e.to_string()))?;
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    bytes[6] = (bytes[6] & 15) | 0x80;
    bytes[8] = (bytes[8] & 63) | 0x80;
    Id::new(uuid::Uuid::from_bytes(bytes).to_string())
        .map_err(|e| StoreError::Invalid(e.to_string()))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityKind {
    Checkpoint,
    Cell,
    Work,
    Run,
    StartAttempt,
    Mandate,
    PartAttempt,
    Permit,
    Resource,
    InterventionCase,
    ProcedureRecord,
    Clearance,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeKind {
    Created,
    Changed,
    SnapshotSeed,
    EvidenceRecorded,
    ReceiptRecorded,
    IntegrityDisputed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlChange {
    pub entity_kind: EntityKind,
    pub change: ChangeKind,
    pub entity: Record,
    pub evidence_ids: Vec<Id>,
}
fn entity_kind(schema: &str) -> Option<EntityKind> {
    Some(match schema {
        "rx.internal.clearance.v1" => EntityKind::Clearance,
        "rx.internal.cell.v1" => EntityKind::Cell,
        "rx.internal.work.v1" => EntityKind::Work,
        "rx.internal.run.v1" => EntityKind::Run,
        "rx.internal.start-attempt.v1" => EntityKind::StartAttempt,
        "rx.internal.mandate.v1" => EntityKind::Mandate,
        "rx.internal.part-attempt.v1" => EntityKind::PartAttempt,
        "rx.internal.permit.v1" => EntityKind::Permit,
        "rx.internal.resource.v1" => EntityKind::Resource,
        "rx.internal.intervention-case.v1" => EntityKind::InterventionCase,
        "rx.internal.case-acknowledgment.v1" | "rx.internal.procedure-record.v1" => {
            EntityKind::ProcedureRecord
        }
        "rx.internal.process-checkpoint.v1" => EntityKind::Checkpoint,
        _ => return None,
    })
}
fn key_kind(key: &Name) -> Option<EntityKind> {
    Some(match key.as_str().split('/').next()? {
        "case" => EntityKind::InterventionCase,
        "caseack" | "procedurerecord" => EntityKind::ProcedureRecord,
        "clearance" => EntityKind::Clearance,
        "cell" => EntityKind::Cell,
        "work" => EntityKind::Work,
        "run" => EntityKind::Run,
        "attempt" => EntityKind::StartAttempt,
        "mandate" => EntityKind::Mandate,
        "part" => EntityKind::PartAttempt,
        "permit" => EntityKind::Permit,
        "resource" => EntityKind::Resource,
        "checkpoint" => EntityKind::Checkpoint,
        _ => return None,
    })
}
/// Post-capture access can read projections and save request results, but cannot mutate core state.
pub(crate) struct Finalized<'a> {
    tx: &'a mut dyn Transaction,
}
impl Finalized<'_> {
    pub fn get(&mut self, key: &Name) -> Result<Option<Record>> {
        self.tx.get(key)
    }
    pub fn remember(&mut self, scope: &RequestScope, value: &SavedRequest) -> Result<()> {
        self.tx.remember(scope, value)
    }
}
pub(crate) struct Journaled<R> {
    inner: R,
}
impl<R: Repository> Journaled<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }
    pub fn into_inner(self) -> R {
        self.inner
    }
    pub fn transact_at_control_cut<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> Result<T>,
    ) -> Result<(T, Counter)> {
        self.transact_finalized(operation, |_, value, sequence| Ok((value, sequence)))
    }
    pub fn transact_finalized<T, U>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> Result<T>,
        finish: impl FnOnce(&mut Finalized<'_>, T, Counter) -> Result<U>,
    ) -> Result<U> {
        self.inner.transact(|tx| {
            let seed = tx.get(&name("control-journal/initialized"))?.is_none();
            let checkpoint_seed = tx.get(&name("checkpoint-artifacts/initialized"))?.is_none();
            let mut tracking = Tracking {
                tx,
                changed: BTreeMap::new(),
                original: BTreeMap::new(),
                receipts: BTreeSet::new(),
                evidence: BTreeMap::new(),
            };
            if seed {
                for prefix in [
                    "cell/",
                    "work/",
                    "run/",
                    "attempt/",
                    "mandate/",
                    "part/",
                    "permit/",
                    "resource/",
                    "checkpoint/",
                    "case/",
                    "caseack/",
                    "procedurerecord/",
                    "clearance/",
                ] {
                    for record in tracking.tx.scan(prefix)? {
                        tracking.changed.insert(record.key.clone(), record);
                    }
                }
            }
            if checkpoint_seed {
                for record in tracking.tx.scan("run/")? {
                    tracking.changed.insert(record.key.clone(), record);
                }
            }
            let result = operation(&mut tracking)?;
            tracking.finish(seed)?;
            if checkpoint_seed {
                tracking.tx.put(
                    &name("checkpoint-artifacts/initialized"),
                    None,
                    &doc(
                        "rx.internal.checkpoint-artifact-version.v1",
                        &crate::checkpoint_artifact::SCHEMA,
                    )?,
                )?;
            }
            if seed {
                tracking.tx.put(
                    &name("control-journal/initialized"),
                    None,
                    &doc("rx.internal.control-journal-version.v1", &VIEW_ID)?,
                )?;
            }
            let sequence = tracking.tx.control_head()?;
            finish(&mut Finalized { tx: tracking.tx }, result, sequence)
        })
    }
}
impl<R: Repository> Repository for Journaled<R> {
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox_after(after, limit)
    }
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> Result<T>,
    ) -> Result<T> {
        self.transact_at_control_cut(operation)
            .map(|(result, _)| result)
    }
    fn pending_outbox(&mut self, limit: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox(limit)
    }
    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>> {
        self.inner.events_after(after, limit)
    }
    fn journal_head(&mut self) -> Result<Counter> {
        self.inner.journal_head()
    }
    fn control_events_after(&mut self, after: Counter, limit: usize) -> Result<Vec<StoredEvent>> {
        self.inner.control_events_after(after, limit)
    }
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
}
struct Tracking<'a> {
    tx: &'a mut dyn Transaction,
    changed: BTreeMap<Name, Record>,
    original: BTreeMap<Name, Option<Record>>,
    receipts: BTreeSet<Id>,
    evidence: BTreeMap<Id, BTreeSet<Id>>,
}
impl Tracking<'_> {
    fn finish(&mut self, seed: bool) -> Result<()> {
        for record in self.changed.values() {
            let Some(kind) = entity_kind(record.document.schema.as_str()) else {
                continue;
            };
            let original = self.original.get(&record.key).and_then(Option::as_ref);
            let mut change = if seed && !self.original.contains_key(&record.key) {
                ChangeKind::SnapshotSeed
            } else if self.original.get(&record.key) == Some(&None) {
                ChangeKind::Created
            } else {
                ChangeKind::Changed
            };
            let mut evidence_ids = vec![];
            if kind == EntityKind::Work {
                let work: Work = decode(record, "rx.internal.work.v1")?;
                if let Some(evidence) = self.evidence.get(work.operation.id()) {
                    change = ChangeKind::EvidenceRecorded;
                    evidence_ids = evidence.iter().cloned().collect();
                } else if self.receipts.contains(work.operation.id()) {
                    change = ChangeKind::ReceiptRecorded;
                }
                if let Some(old) = original {
                    let old: Work = decode(old, "rx.internal.work.v1")?;
                    if old.operation.integrity() != work.operation.integrity() {
                        change = ChangeKind::IntegrityDisputed;
                    }
                }
            }
            let projection = if kind == EntityKind::Run {
                crate::checkpoint_artifact::capture_run(self.tx, record)?
            } else {
                record.clone()
            };
            let value = ControlChange {
                entity_kind: kind,
                change,
                entity: projection.clone(),
                evidence_ids,
            };
            let sequence =
                self.tx
                    .append_control(&id(), &projection, &doc(CHANGE_SCHEMA, &value)?)?;
            if kind == EntityKind::Work && change == ChangeKind::Created {
                let work: Work = decode(record, "rx.internal.work.v1")?;
                if work.operation.phase() != rx_domain::operation::Phase::Admitted
                    || work.operation.revision() != Counter(1)
                {
                    return Err(StoreError::Integrity(
                        "new work does not have its admission state".into(),
                    ));
                }
                let row =
                    self.tx
                        .get(&name("installation/current"))?
                        .ok_or(StoreError::Integrity(
                            "installation missing at admission".into(),
                        ))?;
                let installation: Installation = decode(&row, "rx.internal.installation.v1")?;
                let receipt = AdmissionReceipt {
                    operation: work.operation.id().clone(),
                    intent_digest: work
                        .intent
                        .digest()
                        .map_err(|e| StoreError::Invalid(e.to_string()))?,
                    operation_revision: work.operation.revision(),
                    journal: journal_id(&installation)?,
                    sequence,
                };
                self.tx.put(
                    &key("admissionreceipt", work.operation.id()),
                    None,
                    &doc(ADMISSION_SCHEMA, &receipt)?,
                )?;
            }
        }
        Ok(())
    }
}
impl Transaction for Tracking<'_> {
    fn put(&mut self, k: &Name, expected: Option<Counter>, document: &Document) -> Result<Record> {
        if let Some(expected_kind) = key_kind(k)
            && entity_kind(document.schema.as_str()) != Some(expected_kind)
        {
            return Err(StoreError::Integrity(
                "control entity key/schema mismatch".into(),
            ));
        }
        if entity_kind(document.schema.as_str()).is_some() && !self.original.contains_key(k) {
            self.original.insert(k.clone(), self.tx.get(k)?);
        }
        let record = self.tx.put(k, expected, document)?;
        if entity_kind(document.schema.as_str()).is_some() {
            self.changed.insert(k.clone(), record.clone());
        }
        if document.schema.as_str() == "rx.internal.native-evidence.v1"
            && k.as_str().starts_with("evidence/")
        {
            let value: NativeEvidence = decode(&record, "rx.internal.native-evidence.v1")?;
            self.evidence
                .entry(value.operation)
                .or_default()
                .insert(value.id);
        }
        Ok(record)
    }
    fn append(&mut self, event_id: &Id, document: &Document) -> Result<Counter> {
        if document.schema.as_str() == "rx.event.host-receipt-recorded.v1" {
            let receipt: HostReceipt = canonical::decode_json(
                &canonical::bytes(&document.value)
                    .map_err(|e| StoreError::Invalid(e.to_string()))?,
            )
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
            self.receipts.insert(receipt.operation);
        }
        self.tx.append(event_id, document)
    }
    fn control_head(&mut self) -> Result<Counter> {
        self.tx.control_head()
    }
    fn append_control(&mut self, _: &Id, _: &Record, _: &Document) -> Result<Counter> {
        Err(StoreError::Invalid(
            "only the control-journal unit of work appends control events".into(),
        ))
    }
    fn outbox(&mut self, id: &Id) -> Result<Option<OutboxRecord>> {
        self.tx.outbox(id)
    }
    fn scan(&mut self, prefix: &str) -> Result<Vec<Record>> {
        self.tx.scan(prefix)
    }
    fn get(&mut self, k: &Name) -> Result<Option<Record>> {
        self.tx.get(k)
    }
    fn lookup(&mut self, s: &RequestScope) -> Result<Option<SavedRequest>> {
        self.tx.lookup(s)
    }
    fn remember(&mut self, s: &RequestScope, r: &SavedRequest) -> Result<()> {
        self.tx.remember(s, r)
    }
    fn enqueue(&mut self, id: &Id, d: &Document) -> Result<()> {
        self.tx.enqueue(id, d)
    }
    fn transition_outbox(
        &mut self,
        id: &Id,
        expected: OutboxState,
        next: OutboxState,
    ) -> Result<()> {
        self.tx.transition_outbox(id, expected, next)
    }
}
