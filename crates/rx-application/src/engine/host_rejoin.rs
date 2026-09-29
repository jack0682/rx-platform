use super::*;
use crate::{host_binding_baseline as baseline, host_invalidation as origin, host_rejoin as r};
mod binding;
mod proposal;
mod reads;
mod rebind;
mod settlement;
fn same<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(canonical::bytes(a).map_err(domain_error)? == canonical::bytes(b).map_err(domain_error)?)
}
pub(super) fn build(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: TimePoint,
    identity: &Identity,
    host: &Name,
    origin_cell: &Name,
) -> Result<r::Context> {
    let actor = authorize(
        tx,
        identity,
        meta,
        &now,
        Some(origin_cell),
        Role::ReleaseManager,
        true,
    )?;
    lifecycle::require_serving(tx)?;
    let installation = tx
        .get(&name("installation/current"))?
        .ok_or(StoreError::Integrity("installation missing".into()))?;
    let actual: Installation = decode(&installation, "rx.internal.installation.v1")?;
    if !same(&actual, meta)? {
        return reject(Reject::ContinuityUnproven);
    }
    let (_, first): (_, Cell) = load(tx, "cell", origin_cell, CELL)?;
    if !first.configuration.hosts.contains(host) {
        return reject(Reject::Forbidden);
    }
    let impact = process_change::prospective_impact(
        tx,
        origin_cell,
        &BTreeSet::from([host.clone()]),
        &BTreeSet::new(),
    )?;
    if impact.cells.iter().any(|c| !actor.cells.contains(&c.id)) {
        return reject(Reject::Forbidden);
    }
    if impact.cells.is_empty() || impact.cells.len() > crate::host_recovery::MAX_CELLS {
        return reject(Reject::InvalidInput);
    }
    let (producer_revision, producer): (_, EvidenceProducer) =
        load(tx, "producer", host, "rx.internal.evidence-producer.v1")?;
    if producer.principal != *host {
        return reject(Reject::ContinuityUnproven);
    }
    authorize(
        tx,
        &Identity {
            principal: host.clone(),
            session: producer.session.clone(),
            terminal: None,
        },
        meta,
        &now,
        None,
        Role::Host,
        false,
    )?;
    let replacement = origin::load(tx, &producer.session)?;
    let mut blockers = Vec::new();
    let transport = match host_recovery::registered_transport(tx, meta, host) {
        Ok(value) => Some(value),
        Err(StoreError::Rejected(Reject::NotFound | Reject::CapabilityMissing)) => {
            blockers.push(r::Blocker::TransportMissing);
            None
        }
        Err(error) => return Err(error),
    };
    let ids: BTreeSet<_> = impact.cells.iter().map(|c| c.id.clone()).collect();
    if let Some(value) = &replacement {
        if value.installation != meta.id
            || value.store_generation != meta.store_generation
            || value.runtime_boot != meta.runtime_boot
            || value.after != origin::ProducerIdentity::from(&producer)
        {
            blockers.push(r::Blocker::OriginMismatch);
        }
        if value.previous_runtime_boot != meta.runtime_boot {
            blockers.push(r::Blocker::RuntimeAlsoChanged);
        }
        if value.before.journal != value.after.journal
            || value.before.authentication_binding != value.after.authentication_binding
        {
            blockers.push(r::Blocker::SourceIdentityChanged);
        }
        if value.cells.keys().cloned().collect::<BTreeSet<_>>() != ids {
            blockers.push(r::Blocker::CohortChanged);
        }
    } else {
        blockers.push(r::Blocker::OriginMissing);
    }
    let mut cells = BTreeMap::new();
    for id in ids {
        let (revision, cell): (_, Cell) = load(tx, "cell", &id, CELL)?;
        let mut registration = None;
        let mut baseline = None;
        let mut baseline_digest = None;
        if let Some(value) = &replacement {
            if let Some(cut) = value.cells.get(&id) {
                let current = crate::runtime_invalidation::boundary(revision, &cell)?;
                if current.configuration_digest != cut.after.configuration_digest {
                    blockers.push(r::Blocker::ConfigurationChanged { cell: id.clone() });
                }
                let block_current = cell
                    .blocks
                    .iter()
                    .find(|b| b.id == cut.block.id)
                    .map(|b| same(b, &cut.block))
                    .transpose()?
                    .unwrap_or(false);
                if current != cut.after || !block_current {
                    blockers.push(r::Blocker::ContextAdvanced { cell: id.clone() });
                }
            } else {
                blockers.push(r::Blocker::CohortChanged);
            }
        }
        if cell.configuration.hosts.contains(host) {
            if let Some(row) = tx.get(&key("host", (&id, host)))? {
                let reg: HostRegistration = decode(&row, HOST)?;
                let unchanged = replacement
                    .as_ref()
                    .and_then(|v| v.registrations.get(&id))
                    .map(|old| same(old, &reg))
                    .transpose()?
                    .unwrap_or(false);
                if !unchanged {
                    blockers.push(r::Blocker::RegistrationChanged { cell: id.clone() });
                }
                registration = Some(reg);
            } else {
                blockers.push(r::Blocker::RegistrationMissing { cell: id.clone() });
            }
            if let Some(row) = tx.get(&key("host-link-current", (host, &id)))? {
                let plan: Id = decode(&row, "rx.internal.host-link-id.v1")?;
                baseline = baseline::load(tx, &plan)?;
            }
            if let Some(base) = &baseline {
                baseline_digest = Some(base.digest().map_err(StoreError::Integrity)?);
                let matches = if let (Some(value), Some(reg), Some(pin)) =
                    (&replacement, &registration, &transport)
                {
                    base.host == *host
                        && base.cell == id
                        && base.installation == meta.id
                        && base.store_generation == meta.store_generation
                        && base.runtime_boot == value.previous_runtime_boot
                        && base.host_boot == value.before.boot
                        && base.producer_session == value.before.session
                        && base.evidence_journal == value.before.journal
                        && base.producer_authentication_binding
                            == value.before.authentication_binding
                        && base.transport == *pin
                        && base.delivery_journal == reg.delivery_journal
                        && base.source_sessions == reg.source_sessions
                        && reg.boot_id == value.before.boot
                        && reg.session == value.before.session
                        && producer.cells.get(&id) == Some(&cell.configuration.definition.sha256)
                } else {
                    false
                };
                if !matches {
                    blockers.push(r::Blocker::BaselineMismatch { cell: id.clone() });
                }
            } else {
                blockers.push(r::Blocker::BaselineMissing { cell: id.clone() });
            }
        }
        if host_recovery::local_live_authority(tx, &id)? {
            blockers.push(r::Blocker::LiveAuthority { cell: id.clone() });
        }
        cells.insert(
            id,
            r::CellContext {
                revision,
                cell,
                registration,
                baseline,
                baseline_digest,
            },
        );
    }
    let replacement_digest = replacement
        .as_ref()
        .map(|v| v.digest().map_err(StoreError::Integrity))
        .transpose()?;
    Ok(r::Context {
        schema: name("rx.host-rejoin-context.v1"),
        clock_id: meta.clock_id.clone(),
        installation: meta.id.clone(),
        store_generation: meta.store_generation.clone(),
        runtime_boot: meta.runtime_boot.clone(),
        host: host.clone(),
        origin: origin_cell.clone(),
        producer_revision,
        producer,
        replacement,
        replacement_digest,
        transport,
        cells,
        local_prerequisites_current: blockers.is_empty(),
        blockers,
        fresh_host_read_required: true,
        operation_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn host_rejoin_context(
        &mut self,
        identity: &Identity,
        host: &Name,
        origin: &Name,
    ) -> Result<r::Context> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository
            .transact(|tx| build(tx, meta, clock.now(), identity, host, origin))
    }
}

pub(super) fn rebind_record(tx: &mut dyn Transaction, id: &Id) -> Result<(r::Rebind, r::Proposal)> {
    rebind::load_rebind(tx, id)
}
pub(super) fn rebind_current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    b: &r::Rebind,
    p: &r::Proposal,
) -> Result<bool> {
    rebind::bound_current(tx, meta, b, p)
}
