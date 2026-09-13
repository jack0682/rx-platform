use super::*;
use crate::observation::{BatchReceipt, Disposition, Entry, HostRead};

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn report_fact(&mut self, identity: &Identity, fact: FactRecord) -> Result<()> {
        let receipt = self.report_facts(identity, vec![fact])?;
        match receipt.entries[0].disposition {
            Disposition::IntegrityConflict => Err(StoreError::KeyConflict),
            Disposition::GenerationChanged => reject(Reject::ContinuityUnproven),
            _ => Ok(()),
        }
    }
    /// A single cell's source cut is committed and evaluated as one transaction.
    pub fn report_facts(
        &mut self,
        identity: &Identity,
        facts: Vec<FactRecord>,
    ) -> Result<BatchReceipt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let cell_id = &facts
                .first()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                .cell;
            let principal = authorize(tx, identity, meta, &now, Some(cell_id), Role::Host, false)?;
            let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (cell_id, &principal.id), HOST)?;
            accept_facts(tx, &cell, &host, facts, &now)
        })
    }
    /// Only the pinned transport adapter invokes this after read-binding verification.
    pub fn ingest_host_read(&mut self, input: HostRead) -> Result<BatchReceipt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let snapshot = input.snapshot;
            snapshot.validate().map_err(domain_error)?;
            let (_, plan): (_, crate::host_link::Plan) = load(
                tx,
                "host-link-plan",
                &input.plan,
                "rx.internal.host-link-plan.v1",
            )?;
            let (_, producer): (_, EvidenceProducer) = load(
                tx,
                "producer",
                &plan.host,
                "rx.internal.evidence-producer.v1",
            )?;
            authorize(
                tx,
                &Identity {
                    principal: plan.host.clone(),
                    session: plan.producer_session.clone(),
                    terminal: None,
                },
                meta,
                &now,
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            let (_, cell): (_, Cell) = load(tx, "cell", &plan.cell, CELL)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&plan.cell, &plan.host), HOST)?;
            if !plan.bound
                || snapshot.host != plan.host
                || snapshot.cell != plan.cell
                || producer.session != plan.producer_session
                || producer.peer_boot != snapshot.host_boot
                || producer.journal != snapshot.evidence_journal
                || snapshot.host_boot != plan.host_boot
                || snapshot.delivery_journal != plan.delivery_journal
                || snapshot.evidence_journal != plan.evidence_journal
                || host.id != plan.host
                || host.boot_id != snapshot.host_boot
                || host.delivery_journal != snapshot.delivery_journal
                || host.session != producer.session
                || producer.cells.get(&plan.cell) != Some(&cell.configuration.definition.sha256)
                || snapshot.definition != cell.configuration.definition.sha256
                || snapshot.envelope != cell.configuration.envelope.sha256
                || snapshot.environment.as_str()
                    != match cell.configuration.environment {
                        Environment::Simulation => "SIMULATION",
                        Environment::Physical => "PHYSICAL",
                    }
            {
                return reject(Reject::ContinuityUnproven);
            }
            if snapshot.epoch > cell.epoch
                || snapshot.scopes.keys().collect::<BTreeSet<_>>()
                    != cell.scope_epochs.keys().collect()
                || snapshot
                    .scopes
                    .iter()
                    .any(|(s, e)| cell.scope_epochs.get(s).is_none_or(|p| e > p))
            {
                return reject(Reject::StaleEpoch);
            }
            if !snapshot.sources_available
                || now.age_ns(&input.read_started).is_none()
                || now.age_ns(&snapshot.captured_at).is_none()
                || snapshot.captured_at.ticks_ns < input.read_started.ticks_ns
            {
                return reject(Reject::ConditionUnknown);
            }
            let specs: BTreeMap<_, _> = cell
                .configuration
                .fact_specs
                .iter()
                .filter(|s| s.host == plan.host)
                .map(|s| (&s.id, s))
                .collect();
            if snapshot.observations.len() != specs.len() {
                return reject(Reject::InvalidInput);
            }
            let facts = snapshot
                .observations
                .into_iter()
                .map(|source| {
                    let spec = specs
                        .get(&source.source)
                        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
                    Ok(FactRecord {
                        cell: plan.cell.clone(),
                        id: source.source,
                        source_host: plan.host.clone(),
                        source_generation: source.generation,
                        schema: source.schema,
                        unit: source.unit,
                        acquired_at: source.acquired_at,
                        maximum_age_ns: spec.maximum_age_ns,
                        acquisition_uncertainty_ns: source.uncertainty_ns,
                        quality_good: source.quality_good,
                        origin_age_bounded: source.origin_age_bounded,
                        disputed: source.disputed,
                        value: source.value,
                        evidence_id: source.evidence_id,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            if facts.is_empty() {
                return Ok(BatchReceipt {
                    cell: plan.cell,
                    received_at: now,
                    entries: vec![],
                    maintained_revoked: vec![],
                });
            }
            accept_facts(tx, &cell, &host, facts, &now)
        })
    }
    /// Local writer watchdog. An absence of new observations cannot extend their validity.
    pub fn check_maintained_conditions(&mut self) -> Result<Vec<Name>> {
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let cells = tx
                .scan("cell/")?
                .iter()
                .map(|r| decode::<Cell>(r, CELL))
                .collect::<Result<Vec<_>>>()?;
            let mut invalidated = BTreeSet::new();
            for cell in cells {
                if !invalidated.contains(&cell.configuration.id)
                    && maintained_lost(tx, &cell, &now)?
                {
                    invalidated.extend(invalidate_closure(
                        tx,
                        &cell.configuration.id,
                        BlockReason::ConditionLost,
                    )?);
                }
            }
            Ok(invalidated.into_iter().collect())
        })
    }
    pub fn inspect_fact(
        &mut self,
        identity: &Identity,
        cell: &Name,
        fact: &Name,
    ) -> Result<FactRecord> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            authorize_read(tx, identity, meta, &now, cell)?;
            Ok(load::<FactRecord>(tx, "fact", (cell, fact), FACT)?.1)
        })
    }
}
pub(super) fn accept_facts(
    tx: &mut dyn Transaction,
    cell: &Cell,
    host: &HostRegistration,
    facts: Vec<FactRecord>,
    now: &TimePoint,
) -> Result<BatchReceipt> {
    if facts.is_empty()
        || facts.len() > 128
        || facts.iter().map(|f| &f.id).collect::<BTreeSet<_>>().len() != facts.len()
        || facts
            .iter()
            .map(|f| &f.evidence_id)
            .collect::<BTreeSet<_>>()
            .len()
            != facts.len()
    {
        return reject(Reject::InvalidInput);
    }
    // Validate the entire envelope before any write, including the last source.
    for fact in &facts {
        let spec = cell
            .configuration
            .fact_specs
            .iter()
            .find(|s| s.id == fact.id)
            .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
        if fact.cell != cell.configuration.id
            || spec.host != host.id
            || fact.source_host != host.id
            || spec.schema != fact.schema
            || spec.unit != fact.unit
            || fact.maximum_age_ns != spec.maximum_age_ns
            || fact.acquisition_uncertainty_ns > spec.maximum_uncertainty_ns
        {
            return reject(Reject::InvalidInput);
        }
    }
    let mut entries = vec![];
    for fact in &facts {
        entries.push(record_fact(tx, host, fact.clone(), now)?);
    }
    let mut revoked = BTreeSet::new();
    if maintained_lost(tx, cell, now)? {
        // Include cells that depend on the same sources, once per resource/scope closure.
        for row in tx.scan("cell/")? {
            let other: Cell = decode(&row, CELL)?;
            if !revoked.contains(&other.configuration.id)
                && other.configuration.fact_specs.iter().any(|s| {
                    facts
                        .iter()
                        .any(|f| s.id == f.id && s.host == f.source_host)
                })
            {
                revoked.extend(invalidate_closure(
                    tx,
                    &other.configuration.id,
                    BlockReason::ConditionLost,
                )?);
            }
        }
    }
    Ok(BatchReceipt {
        cell: cell.configuration.id.clone(),
        received_at: now.clone(),
        entries,
        maintained_revoked: revoked.into_iter().collect(),
    })
}
fn maintained_lost(tx: &mut dyn Transaction, cell: &Cell, now: &TimePoint) -> Result<bool> {
    if cell.configuration.maintained_conditions.is_empty() {
        return Ok(false);
    }
    let active = tx
        .scan("run/")?
        .iter()
        .map(|r| decode::<Run>(r, RUN))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .any(|r| {
            r.cell == cell.configuration.id
                && (r.state == RunState::Executing || r.pending_attempt.is_some())
        });
    if !active {
        return Ok(false);
    }
    match evaluate(tx, cell, &cell.configuration.maintained_conditions, now) {
        Ok(_) => Ok(false),
        Err(StoreError::Rejected(Reject::ConditionFailed | Reject::ConditionUnknown)) => Ok(true),
        Err(e) => Err(e),
    }
}
fn record_fact(
    tx: &mut dyn Transaction,
    host: &HostRegistration,
    mut fact: FactRecord,
    now: &TimePoint,
) -> Result<Entry> {
    let result = |disposition| Entry {
        source: fact.id.clone(),
        evidence: fact.evidence_id.clone(),
        disposition,
    };
    let evidence_key = key("factevidence", &fact.evidence_id);
    let evidence = doc("rx.internal.observation-evidence.v1", &fact.evidence())?;
    if let Some(old) = tx.get(&evidence_key)? {
        if old.document != evidence {
            let incident = key(
                "observationincident",
                (
                    &fact.evidence_id,
                    canonical::digest("RX-EVIDENCE-CONFLICT-v1", &evidence)
                        .map_err(domain_error)?,
                ),
            );
            if tx.get(&incident)?.is_none() {
                tx.put(&incident, None, &evidence)?;
                invalidate_fact_dependents(tx, &fact, BlockReason::IntegrityConflict)?;
                event(tx, "rx.event.observation-conflict.v1", &fact)?;
            }
            return Ok(result(Disposition::IntegrityConflict));
        }
    } else {
        tx.put(&evidence_key, None, &evidence)?;
    }
    let changed_generation = host.source_sessions.get(&fact.id) != Some(&fact.source_generation)
        || fact.acquired_at.clock_id != now.clock_id;
    let current_key = key("fact", (&fact.cell, &fact.id));
    let previous = tx.get(&current_key)?;
    let mut already_disputed = false;
    if let Some(record) = &previous {
        let old: FactRecord = decode(record, FACT)?;
        already_disputed = old.disputed && old.source_generation == fact.source_generation;
        if old.evidence_id == fact.evidence_id {
            return Ok(result(if changed_generation {
                Disposition::GenerationChanged
            } else {
                Disposition::Duplicate
            }));
        }
        if !changed_generation
            && old.acquired_at.clock_id == fact.acquired_at.clock_id
            && old.acquired_at.ticks_ns > fact.acquired_at.ticks_ns
        {
            event(tx, "rx.event.historical-observation.v1", &fact)?;
            return Ok(result(Disposition::Historical));
        }
    }
    let entry = result(if changed_generation {
        Disposition::GenerationChanged
    } else {
        Disposition::Current
    });
    if changed_generation {
        fact.quality_good = false;
    }
    tx.put(
        &current_key,
        previous.map(|r| r.revision),
        &doc(FACT, &fact)?,
    )?;
    if changed_generation {
        let incident = key(
            "source-generation-loss",
            (
                &fact.cell,
                &fact.source_host,
                &fact.id,
                &fact.source_generation,
                &fact.acquired_at.clock_id,
                host.source_sessions.get(&fact.id),
            ),
        );
        if tx.get(&incident)?.is_none() {
            tx.put(
                &incident,
                None,
                &doc("rx.internal.source-generation-loss.v1", &fact.evidence())?,
            )?;
            invalidate_fact_dependents(tx, &fact, BlockReason::DeviceRestart)?;
            event(tx, "rx.event.source-generation-changed.v1", &fact)?;
        }
    } else {
        if fact.disputed && !already_disputed {
            invalidate_fact_dependents(tx, &fact, BlockReason::IntegrityConflict)?;
        }
        event(tx, "rx.event.observation-recorded.v1", &fact)?;
    }
    Ok(entry)
}
