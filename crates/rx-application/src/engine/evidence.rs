use super::*;
use rx_domain::operation::{Conclusion, Phase};
const NATIVE_EVIDENCE: &str = "rx.internal.native-evidence.v1";
const EVIDENCE_CURSOR: &str = "rx.internal.evidence-cursor.v1";

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn ingest_evidence(
        &mut self,
        identity: &Identity,
        batch: EvidenceBatch,
    ) -> Result<EvidenceCursor> {
        self.ingest_evidence_commit(identity, batch)
            .map(|commit| commit.producer)
    }
    pub fn ingest_evidence_commit(
        &mut self,
        identity: &Identity,
        batch: EvidenceBatch,
    ) -> Result<EvidenceCommit> {
        self.ingest_evidence_inner(identity, batch, false)
    }
    /// Network Publish must use the explicitly negotiated producer session inside T2.
    pub fn publish_evidence(
        &mut self,
        identity: &Identity,
        batch: EvidenceBatch,
    ) -> Result<EvidenceCommit> {
        self.ingest_evidence_inner(identity, batch, true)
    }
    fn ingest_evidence_inner(
        &mut self,
        identity: &Identity,
        batch: EvidenceBatch,
        transport: bool,
    ) -> Result<EvidenceCommit> {
        if batch.first.0 == 0 || batch.records.len() > 128 {
            return reject(Reject::InvalidInput);
        }
        let clock = &self.clock;
        let meta = &self.installation;
        let (result, control_sequence) = self.repository.transact_at_control_cut(|tx| {
            let now = clock.now();
            let principal = authorize(tx, identity, meta, &now, None, Role::Host, false)?;
            if transport {
                let (_, producer): (_, EvidenceProducer) = load(
                    tx,
                    "producer",
                    &principal.id,
                    "rx.internal.evidence-producer.v1",
                )?;
                if producer.session != identity.session || producer.journal != batch.journal {
                    return reject(Reject::Unauthenticated);
                }
                if producer.cells.is_empty() {
                    return reject(Reject::CapabilityMissing);
                }
                if !producer
                    .cells
                    .keys()
                    .any(|cell| principal.cells.contains(cell))
                {
                    return reject(Reject::Forbidden);
                }
                for evidence in &batch.records {
                    if evidence.native_details.is_none() {
                        return reject(Reject::InvalidInput);
                    }
                    if let Some(work) = tx.get(&key("work", &evidence.operation))? {
                        let work: Work = decode(&work, WORK)?;
                        if !producer.cells.contains_key(&work.cell) {
                            return reject(Reject::CapabilityMissing);
                        }
                    }
                }
            }
            let source_key = key("evidencesource", &principal.id);
            if let Some(source) = tx.get(&source_key)? {
                let declared: Id = decode(&source, "rx.internal.evidence-source.v1")?;
                if declared != batch.journal {
                    let incident = key("journalchange", (&principal.id, &batch.journal));
                    if tx.get(&incident)?.is_none() {
                        tx.put(
                            &incident,
                            None,
                            &doc("rx.internal.journal-change.v1", &batch)?,
                        )?;
                        invalidate_host_cells(tx, &principal.id)?;
                        event(tx, "rx.event.evidence-journal-changed.v1", &batch.journal)?;
                    }
                    return Ok(Err(StoreError::Integrity(
                        "journal changed; explicit gap reconciliation required".into(),
                    )));
                }
            } else {
                tx.put(
                    &source_key,
                    None,
                    &doc("rx.internal.evidence-source.v1", &batch.journal)?,
                )?;
            }
            let cursor_key = key("evidencecursor", (&principal.id, &batch.journal));
            let previous = tx.get(&cursor_key)?;
            let mut cursor = if let Some(row) = &previous {
                decode::<EvidenceCursor>(row, EVIDENCE_CURSOR)?
            } else {
                EvidenceCursor {
                    journal: batch.journal.clone(),
                    through: Counter(0),
                    disputed: false,
                }
            };
            if cursor.disputed {
                return Err(StoreError::Integrity("evidence stream is disputed".into()));
            }
            if batch.first.0
                > cursor
                    .through
                    .0
                    .checked_add(1)
                    .ok_or(StoreError::Integrity("cursor overflow".into()))?
            {
                return Err(StoreError::Invalid(format!(
                    "GAP: next expected {}",
                    cursor.through.0 + 1
                )));
            }
            for (index, evidence) in batch.records.iter().enumerate() {
                let owner_key = key("evidenceowner", &evidence.id);
                if let Some(row) = tx.get(&owner_key)? {
                    if decode::<Name>(&row, "rx.internal.evidence-owner.v1")? != principal.id {
                        return reject(Reject::Forbidden);
                    }
                } else {
                    tx.put(
                        &owner_key,
                        None,
                        &doc("rx.internal.evidence-owner.v1", &principal.id)?,
                    )?;
                }
                let sequence = Counter(
                    batch
                        .first
                        .0
                        .checked_add(index as u64)
                        .ok_or(StoreError::Invalid("batch sequence overflow".into()))?,
                );
                let source = key("evidenceslot", (&principal.id, &batch.journal, sequence));
                let incoming = doc(NATIVE_EVIDENCE, evidence)?;
                if sequence <= cursor.through {
                    let existing = tx.get(&source)?.ok_or(StoreError::Integrity(
                        "missing committed evidence slot".into(),
                    ))?;
                    if existing.document != incoming {
                        cursor.disputed = true;
                        tx.put(
                            &key(
                                "evidenceconflict",
                                (&principal.id, &batch.journal, sequence, evidence.id.clone()),
                            ),
                            None,
                            &incoming,
                        )?;
                        dispute_affected(tx, &principal.id, &evidence.operation)?;
                        tx.put(
                            &cursor_key,
                            previous.as_ref().map(|r| r.revision),
                            &doc(EVIDENCE_CURSOR, &cursor)?,
                        )?;
                        return Ok(Err(StoreError::Integrity(
                            "INTEGRITY_CONFLICT: same journal sequence".into(),
                        )));
                    }
                    continue;
                }
                if let Some(existing) = tx.get(&key("evidence", &evidence.id))? {
                    if existing.document != incoming {
                        cursor.disputed = true;
                        dispute_affected(tx, &principal.id, &evidence.operation)?;
                        tx.put(
                            &cursor_key,
                            previous.as_ref().map(|r| r.revision),
                            &doc(EVIDENCE_CURSOR, &cursor)?,
                        )?;
                        return Ok(Err(StoreError::Integrity(
                            "INTEGRITY_CONFLICT: same evidence identity".into(),
                        )));
                    }
                } else {
                    tx.put(&key("evidence", &evidence.id), None, &incoming)?;
                }
                tx.put(&source, None, &incoming)?;
                apply_native(tx, &principal, evidence, &now)?;
                cursor.through = sequence;
            }
            tx.put(
                &cursor_key,
                previous.map(|r| r.revision),
                &doc(EVIDENCE_CURSOR, &cursor)?,
            )?;
            tx.append(
                &id(),
                &doc("rx.event.evidence-batch-committed.v1", &cursor)?,
            )?;
            Ok(Ok(cursor))
        })?;
        result.map(|producer| EvidenceCommit {
            producer,
            installation: meta.id.clone(),
            store_generation: meta.store_generation.clone(),
            platform_sequence: control_sequence,
        })
    }
    pub fn inspect_native_evidence(
        &mut self,
        identity: &Identity,
        evidence: &Id,
    ) -> Result<NativeEvidence> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let (_, owner): (_, Name) = load(
                tx,
                "evidenceowner",
                evidence,
                "rx.internal.evidence-owner.v1",
            )?;
            if owner != p.id {
                return reject(Reject::Forbidden);
            }
            let (_, value): (_, NativeEvidence) = load(tx, "evidence", evidence, NATIVE_EVIDENCE)?;
            if let Some(record) = tx.get(&key("work", &value.operation))? {
                let work: Work = decode(&record, WORK)?;
                if !p.cells.contains(&work.cell) {
                    return reject(Reject::Forbidden);
                }
            }
            Ok(value)
        })
    }
}

pub(super) fn reapply_correlated_native(
    tx: &mut dyn Transaction,
    principal: &Principal,
    operation: &Id,
    now: &TimePoint,
) -> Result<()> {
    for record in tx.scan("evidence/")? {
        if record.document.schema.as_str() != NATIVE_EVIDENCE {
            continue;
        }
        let evidence: NativeEvidence = decode(&record, NATIVE_EVIDENCE)?;
        if &evidence.operation == operation {
            let (_, owner): (_, Name) = load(
                tx,
                "evidenceowner",
                &evidence.id,
                "rx.internal.evidence-owner.v1",
            )?;
            if owner != principal.id {
                return reject(Reject::Forbidden);
            }
            apply_native(tx, principal, &evidence, now)?;
        }
    }
    Ok(())
}

fn apply_native(
    tx: &mut dyn rx_ports::Transaction,
    principal: &Principal,
    evidence: &NativeEvidence,
    now: &TimePoint,
) -> Result<()> {
    let record = tx.get(&key("work", &evidence.operation))?;
    let Some(record) = record else {
        event(tx, "rx.event.orphan-evidence.v1", evidence)?;
        dispute_affected(tx, &principal.id, &evidence.operation)?;
        return Ok(());
    };
    let mut work: Work = decode(&record, WORK)?;
    if work.host != principal.id || !principal.cells.contains(&work.cell) {
        return reject(Reject::Forbidden);
    }
    if work.intent.profile_digest != evidence.profile_digest
        || work
            .invocation
            .as_ref()
            .is_some_and(|i| i != &evidence.invocation)
    {
        work.operation.dispute().map_err(domain_error)?;
        save(
            tx,
            "work",
            work.operation.id(),
            Some(record.revision),
            WORK,
            &work,
        )?;
        invalidate_closure(tx, &work.cell, BlockReason::IntegrityConflict)?;
        return Ok(());
    }
    if work.invocation.is_none() {
        work.operation.lose_continuity().map_err(domain_error)?;
        save(
            tx,
            "work",
            work.operation.id(),
            Some(record.revision),
            WORK,
            &work,
        )?;
        event(tx, "rx.event.correlation-awaiting-receipt.v1", evidence)?;
        return Ok(());
    }
    let (_, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
    let (permit_revision, mut permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    // A correlated native result proves native entry even if its RPC receipt arrived later.
    if permit.state == PermitState::Issued {
        permit.state = PermitState::Consumed;
        save(
            tx,
            "permit",
            &permit.id,
            Some(permit_revision),
            PERMIT,
            &permit,
        )?;
    }
    let authorization = super::delivery::key_id(work.operation.id(), "authorize");
    if tx
        .outbox(&authorization)?
        .is_some_and(|row| row.state == rx_ports::OutboxState::New)
    {
        tx.transition_outbox(
            &authorization,
            rx_ports::OutboxState::New,
            rx_ports::OutboxState::Voided,
        )?;
    }
    let continuity =
        cell.epoch == permit.epoch && cell.scope_epochs == permit.scopes && cell.blocks.is_empty();
    let mut proofs = vec![evidence.id.clone()];
    let outcome = match &work.completion {
        CompletionRule::Unobservable => None,
        CompletionRule::NativeOutcomes {
            table,
            postconditions,
        } => {
            use rx_process_contract::native_outcome::NativeConclusion;
            if table.profile_digest != work.intent.profile_digest
                || table.completion_rule != work.intent.completion_rule
            {
                return Err(StoreError::Integrity(
                    "stored outcome table binding differs".into(),
                ));
            }
            match table
                .resolve(&evidence.status_schema, evidence.status)
                .map_err(domain_error)?
            {
                Some(NativeConclusion::Failed) => Some(Outcome::Failed),
                Some(NativeConclusion::Canceled) => Some(Outcome::Canceled),
                Some(NativeConclusion::Succeeded) if postconditions.is_empty() => {
                    Some(Outcome::Succeeded)
                }
                Some(NativeConclusion::Succeeded) if continuity => {
                    match evaluate(tx, &cell, postconditions, now) {
                        Ok(p) => {
                            proofs.extend(p.evidence_ids);
                            Some(Outcome::Succeeded)
                        }
                        Err(StoreError::Rejected(
                            Reject::ConditionFailed | Reject::ConditionUnknown,
                        )) => None,
                        Err(e) => return Err(e),
                    }
                }
                _ => None,
            }
        }
        CompletionRule::Native {
            schema,
            success,
            failure,
            postconditions,
        } => {
            if schema != &evidence.status_schema {
                None
            } else if failure.iter().any(|s| s.0 == evidence.status.0) {
                Some(Outcome::Failed)
            } else if success.iter().any(|s| s.0 == evidence.status.0) {
                if postconditions.is_empty() {
                    Some(Outcome::Succeeded)
                } else if continuity {
                    match evaluate(tx, &cell, postconditions, now) {
                        Ok(p) => {
                            proofs.extend(p.evidence_ids);
                            Some(Outcome::Succeeded)
                        }
                        Err(StoreError::Rejected(
                            Reject::ConditionFailed | Reject::ConditionUnknown,
                        )) => None,
                        Err(e) => return Err(e),
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }
        CompletionRule::Predicate { conditions } => {
            if continuity {
                match evaluate(tx, &cell, conditions, now) {
                    Ok(p) => {
                        proofs.extend(p.evidence_ids);
                        Some(Outcome::Succeeded)
                    }
                    Err(StoreError::Rejected(
                        Reject::ConditionFailed | Reject::ConditionUnknown,
                    )) => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            }
        }
    };
    if let Some(outcome) = outcome {
        work.operation
            .conclude(Conclusion {
                outcome,
                evidence_ids: proofs,
            })
            .map_err(domain_error)?;
    } else if work.operation.phase() != Phase::Settled {
        work.operation.lose_continuity().map_err(domain_error)?;
    }
    let disputed = work.operation.integrity() == Integrity::Disputed;
    save(
        tx,
        "work",
        work.operation.id(),
        Some(record.revision),
        WORK,
        &work,
    )?;
    event(tx, "rx.event.native-evidence-applied.v1", &work)?;
    if disputed {
        invalidate_closure(tx, &work.cell, BlockReason::IntegrityConflict)?;
    }
    Ok(())
}
fn dispute_affected(tx: &mut dyn rx_ports::Transaction, host: &Name, operation: &Id) -> Result<()> {
    if let Some(record) = tx.get(&key("work", operation))? {
        let mut work: Work = decode(&record, WORK)?;
        if &work.host != host {
            return reject(Reject::Forbidden);
        }
        work.operation.dispute().map_err(domain_error)?;
        save(
            tx,
            "work",
            work.operation.id(),
            Some(record.revision),
            WORK,
            &work,
        )?;
        invalidate_closure(tx, &work.cell, BlockReason::IntegrityConflict)?;
    } else {
        invalidate_host_cells(tx, host)?;
    }
    Ok(())
}
fn invalidate_host_cells(tx: &mut dyn rx_ports::Transaction, host: &Name) -> Result<()> {
    let cells = tx
        .scan("cell/")?
        .iter()
        .map(|row| decode::<Cell>(row, CELL))
        .collect::<Result<Vec<_>>>()?;
    let mut visited = BTreeSet::new();
    for cell in cells {
        if cell.configuration.hosts.contains(host) && !visited.contains(&cell.configuration.id) {
            visited.extend(invalidate_closure(
                tx,
                &cell.configuration.id,
                BlockReason::IntegrityConflict,
            )?);
        }
    }
    Ok(())
}
