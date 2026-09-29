use super::*;
use crate::observation_link::{Link, Read, Register};
use rx_domain::host_snapshot::HostSnapshot;
pub(super) const PREFIX: &str = "observation-link";
const SCHEMA: &str = "rx.internal.observation-link.v1";
const CURRENT: &str = "observation-link-current";
const CURRENT_SCHEMA: &str = "rx.internal.observation-link-id.v1";

fn configuration_digest(cell: &Cell) -> Result<Digest> {
    crate::runtime_invalidation::configuration_digest(&cell.configuration)
        .map_err(StoreError::Integrity)
}
fn check_read(
    cell: &Cell,
    snapshot: &HostSnapshot,
    started: &TimePoint,
    now: &TimePoint,
) -> Result<()> {
    snapshot.validate().map_err(domain_error)?;
    if !cell.configuration.hosts.contains(&snapshot.host)
        || cell
            .configuration
            .steps
            .iter()
            .any(|step| step.host == snapshot.host)
        || snapshot.definition != cell.configuration.definition.sha256
        || snapshot.envelope != cell.configuration.envelope.sha256
        || snapshot.environment.as_str()
            != match cell.configuration.environment {
                Environment::Simulation => "SIMULATION",
                Environment::Physical => "PHYSICAL",
            }
        || !snapshot.resource_fences.is_empty()
        || !snapshot.pending_operations.is_empty()
        || !snapshot.pending_permits.is_empty()
    {
        return reject(Reject::CapabilityMissing);
    }
    if snapshot.epoch > cell.epoch
        || snapshot.scopes.keys().collect::<BTreeSet<_>>() != cell.scope_epochs.keys().collect()
        || snapshot
            .scopes
            .iter()
            .any(|(s, e)| cell.scope_epochs.get(s).is_none_or(|current| e > current))
    {
        return reject(Reject::StaleEpoch);
    }
    if !snapshot.sources_available
        || now.age_ns(started).is_none_or(|age| age > 100_000_000)
        || now.age_ns(&snapshot.captured_at).is_none()
        || snapshot.captured_at.ticks_ns < started.ticks_ns
    {
        return reject(Reject::ConditionUnknown);
    }
    let specs: Vec<_> = cell
        .configuration
        .fact_specs
        .iter()
        .filter(|s| s.host == snapshot.host)
        .collect();
    if specs.is_empty()
        || specs.len() != snapshot.observations.len()
        || snapshot.observations.iter().any(|o| {
            !specs
                .iter()
                .any(|s| s.id == o.source && s.schema == o.schema && s.unit == o.unit)
        })
    {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}
fn producer(
    tx: &mut dyn Transaction,
    meta: &Installation,
    snapshot: &HostSnapshot,
    now: &TimePoint,
) -> Result<EvidenceProducer> {
    let (_, p): (_, EvidenceProducer) = load(
        tx,
        "producer",
        &snapshot.host,
        "rx.internal.evidence-producer.v1",
    )?;
    authorize(
        tx,
        &Identity {
            principal: p.principal.clone(),
            session: p.session.clone(),
            terminal: None,
        },
        meta,
        now,
        Some(&snapshot.cell),
        Role::Host,
        false,
    )?;
    if p.principal != snapshot.host
        || p.peer_boot != snapshot.host_boot
        || p.journal != snapshot.evidence_journal
        || p.cells.get(&snapshot.cell) != Some(&snapshot.definition)
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(p)
}
fn load_current(tx: &mut dyn Transaction, id: &Id) -> Result<Link> {
    let (_, link): (_, Link) = load(tx, PREFIX, id, SCHEMA)?;
    let (_, current): (_, Id) = load(tx, CURRENT, (&link.host, &link.cell), CURRENT_SCHEMA)?;
    if current != link.id {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(link)
}
fn check_link(
    link: &Link,
    meta: &Installation,
    p: &EvidenceProducer,
    cell: &Cell,
    snapshot: &HostSnapshot,
) -> Result<()> {
    let original = &link.provenance.host_read;
    if link.installation != meta.id
        || link.store_generation != meta.store_generation
        || link.runtime_boot != meta.runtime_boot
        || link.host != snapshot.host
        || link.cell != snapshot.cell
        || link.producer_session != p.session
        || link.producer_authentication_binding != p.authentication_binding
        || original.host_boot != snapshot.host_boot
        || original.delivery_journal != snapshot.delivery_journal
        || original.evidence_journal != snapshot.evidence_journal
        || link.configuration_digest != configuration_digest(cell)?
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Pinned network adapter only. Does not create a HostRegistration, grant, fence or qualification.
    pub fn register_observation_link(&mut self, input: Register) -> Result<Link> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let snapshot = &input.provenance.host_read;
            if snapshot.host != input.host {
                return reject(Reject::InvalidInput);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &snapshot.cell, CELL)?;
            check_read(&cell, snapshot, &input.read_started, &now)?;
            let p = producer(tx, meta, snapshot, &now)?;
            host_link::baseline::validate_provenance(
                &input.provenance,
                snapshot,
                &input.read_started,
                &cell.configuration,
                &now,
            )?;
            if tx
                .get(&key("host-link-current", (&input.host, &snapshot.cell)))?
                .is_some()
                || tx
                    .get(&key("host", (&snapshot.cell, &input.host)))?
                    .is_some()
            {
                return reject(Reject::CapabilityMissing);
            }
            let source_sessions = snapshot
                .observations
                .iter()
                .map(|o| (o.source.clone(), o.generation.clone()))
                .collect::<BTreeMap<_, _>>();
            // A registration establishes current source continuity; stale samples cannot establish it.
            for source in &snapshot.observations {
                let spec = cell
                    .configuration
                    .fact_specs
                    .iter()
                    .find(|s| s.id == source.source)
                    .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
                if !source.quality_good
                    || !source.origin_age_bounded
                    || source.disputed
                    || source.uncertainty_ns > spec.maximum_uncertainty_ns
                    || now
                        .age_ns(&source.acquired_at)
                        .is_none_or(|age| age > spec.maximum_age_ns.0)
                {
                    return reject(Reject::ConditionUnknown);
                }
            }
            let k = key(CURRENT, (&input.host, &snapshot.cell));
            if let Some(row) = tx.get(&k)? {
                let prior: Id = decode(&row, CURRENT_SCHEMA)?;
                let old = load_current(tx, &prior)?;
                check_link(&old, meta, &p, &cell, snapshot)?;
                if old.source_sessions != source_sessions
                    || old.provenance.transport != input.provenance.transport
                    || old.provenance.configuration.snapshot.binding_digest
                        != input.provenance.configuration.snapshot.binding_digest
                {
                    return reject(Reject::ContinuityUnproven);
                }
                // A read-only TLS reconnect may use a new platform transport session. It changes
                // no source identity or rights and must not rewrite the immutable first provenance.
                return Ok(old);
            }
            let link = Link {
                id: id(),
                host: input.host,
                cell: snapshot.cell.clone(),
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                producer_session: p.session,
                producer_authentication_binding: p.authentication_binding,
                platform_session: input.platform_session,
                configuration_digest: configuration_digest(&cell)?,
                source_sessions,
                created_at: now,
                provenance: input.provenance,
            };
            save(tx, PREFIX, &link.id, None, SCHEMA, &link)?;
            tx.put(&k, None, &doc(CURRENT_SCHEMA, &link.id)?)?;
            event(tx, "rx.event.observation-link-registered.v1", &link)?;
            Ok(link)
        })
    }
    /// Ingestion accepts only a current registered source cut; the fact path grants no action rights.
    pub fn ingest_observation_read(
        &mut self,
        input: Read,
    ) -> Result<crate::observation::BatchReceipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let link = load_current(tx, &input.link)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &link.cell, CELL)?;
            check_read(&cell, &input.snapshot, &input.read_started, &now)?;
            let p = producer(tx, meta, &input.snapshot, &now)?;
            check_link(&link, meta, &p, &cell, &input.snapshot)?;
            let facts = input
                .snapshot
                .observations
                .into_iter()
                .map(|o| {
                    let spec = cell
                        .configuration
                        .fact_specs
                        .iter()
                        .find(|s| s.id == o.source)
                        .expect("validated source");
                    FactRecord {
                        cell: link.cell.clone(),
                        id: o.source,
                        source_host: link.host.clone(),
                        source_generation: o.generation,
                        schema: o.schema,
                        unit: o.unit,
                        acquired_at: o.acquired_at,
                        maximum_age_ns: spec.maximum_age_ns,
                        acquisition_uncertainty_ns: o.uncertainty_ns,
                        quality_good: o.quality_good,
                        origin_age_bounded: o.origin_age_bounded,
                        disputed: o.disputed,
                        value: o.value,
                        evidence_id: o.evidence_id,
                    }
                })
                .collect();
            observation::accept_facts(
                tx,
                &cell,
                observation::SourceBinding {
                    host: &link.host,
                    generations: &link.source_sessions,
                },
                facts,
                &now,
            )
        })
    }
}
/// Invoked in the same transaction that replaces an authenticated producer. No old
/// fact may remain operationally trusted until its usual expiry merely because it had no grant.
pub(super) fn invalidate_producer(
    tx: &mut dyn Transaction,
    host: &Name,
    touched: &mut BTreeSet<Name>,
) -> Result<()> {
    for row in tx.scan(&format!("{CURRENT}/"))? {
        let id: Id = decode(&row, CURRENT_SCHEMA)?;
        let link = load_current(tx, &id)?;
        if &link.host == host && !touched.contains(&link.cell) {
            touched.extend(invalidate_closure(
                tx,
                &link.cell,
                BlockReason::DeviceRestart,
            )?);
            event(tx, "rx.event.observation-link-retired.v1", &link.id)?;
        }
    }
    Ok(())
}
/// Prevent an observation-only relation from being silently promoted into an operating link.
pub(super) fn has_current(tx: &mut dyn Transaction, host: &Name, cell: &Name) -> Result<bool> {
    Ok(tx.get(&key(CURRENT, (host, cell)))?.is_some())
}

/// Registered observation generations for the common evaluator. Missing or retired
/// identity yields no usable generation; storage integrity failures remain errors.
pub(super) fn source_sessions(
    tx: &mut dyn Transaction,
    cell: &Cell,
    host: &Name,
    now: &TimePoint,
) -> Result<Option<BTreeMap<Name, Id>>> {
    let Some(row) = tx.get(&key(CURRENT, (host, &cell.configuration.id)))? else {
        return Ok(None);
    };
    let id: Id = decode(&row, CURRENT_SCHEMA)?;
    let link = load_current(tx, &id)?;
    let meta = tx
        .get(&name("installation/current"))?
        .ok_or_else(|| StoreError::Integrity("installation missing".into()))?;
    let meta: Installation = decode(&meta, "rx.internal.installation.v1")?;
    let validate = (|| {
        let p = producer(tx, &meta, &link.provenance.host_read, now)?;
        check_link(&link, &meta, &p, cell, &link.provenance.host_read)?;
        if cell.configuration.steps.iter().any(|s| s.host == *host) {
            return reject(Reject::CapabilityMissing);
        }
        Ok(())
    })();
    match validate {
        Ok(()) => Ok(Some(link.source_sessions)),
        Err(StoreError::Rejected(_)) => Ok(None),
        Err(e) => Err(e),
    }
}
