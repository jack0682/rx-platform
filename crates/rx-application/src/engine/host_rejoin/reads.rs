use super::*;
use crate::host_recovery::{READ_AGE_NS, READ_WINDOW_NS, ReadEvidence};
fn window(now: &TimePoint, start: &TimePoint, end: &TimePoint) -> Result<()> {
    if end.age_ns(start).is_none_or(|n| n > READ_WINDOW_NS)
        || now.age_ns(start).is_none_or(|n| n > READ_AGE_NS)
    {
        return reject(Reject::ConditionUnknown);
    }
    Ok(())
}
pub(super) fn validate(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    c: &r::Context,
    read: &ReadEvidence,
) -> Result<()> {
    validate_fences(tx, now, c, read, None)
}
pub(super) fn validate_fences(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    c: &r::Context,
    read: &ReadEvidence,
    allowed: Option<&BTreeMap<Name, BTreeSet<Counter>>>,
) -> Result<()> {
    let pin = c
        .transport
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    pin.validate_current_bindings()
        .map_err(StoreError::Integrity)?;
    read.configuration.validate().map_err(StoreError::Invalid)?;
    let host_cells: BTreeSet<_> = c
        .cells
        .iter()
        .filter(|(_, v)| v.cell.configuration.hosts.contains(&c.host))
        .map(|(id, _)| id)
        .collect();
    if &read.transport != pin
        || read.cells.keys().collect::<BTreeSet<_>>() != host_cells
        || read
            .configuration
            .snapshot
            .cells
            .iter()
            .map(|v| &v.cell)
            .collect::<BTreeSet<_>>()
            != host_cells
        || read.configuration.snapshot.host != c.host
        || read.configuration.snapshot.host_boot != c.producer.peer_boot
    {
        return reject(Reject::ContinuityUnproven);
    }
    window(
        now,
        &read.configuration_started,
        &read.configuration_finished,
    )?;
    let works = tx
        .scan("work/")?
        .iter()
        .map(|r| decode::<Work>(r, WORK))
        .collect::<Result<Vec<_>>>()?;
    for id in host_cells {
        let cut = &c.cells[id];
        let config = &cut.cell.configuration;
        let base = cut
            .baseline
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        let actual = &read.cells[id];
        let snapshot = &actual.snapshot;
        snapshot.validate().map_err(domain_error)?;
        window(now, &actual.started, &actual.finished)?;
        let observed = read
            .configuration
            .snapshot
            .cells
            .iter()
            .find(|v| &v.cell == id)
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if actual
            .started
            .age_ns(&read.configuration_finished)
            .is_none()
            || snapshot.captured_at.age_ns(&actual.started).is_none()
            || actual.finished.age_ns(&snapshot.captured_at).is_none()
            || snapshot.host != c.host
            || snapshot.host_boot != c.producer.peer_boot
            || snapshot.delivery_journal != base.delivery_journal
            || snapshot.evidence_journal != base.evidence_journal
            || read.configuration.snapshot.delivery_journal != base.delivery_journal
            || snapshot.definition != config.definition.sha256
            || snapshot.envelope != config.envelope.sha256
            || snapshot.environment.as_str()
                != match config.environment {
                    Environment::Simulation => "SIMULATION",
                    Environment::Physical => "PHYSICAL",
                }
            || !snapshot.sources_available
            || observed.definition != snapshot.definition
            || observed.envelope != snapshot.envelope
            || observed.environment != snapshot.environment
            || observed.epoch != snapshot.epoch
            || observed.scopes != snapshot.scopes
            || observed.blocked.iter().collect::<BTreeSet<_>>()
                != snapshot.block_ids.iter().collect()
            || snapshot.scopes.keys().collect::<BTreeSet<_>>()
                != cut.cell.scope_epochs.keys().collect()
            || snapshot.epoch > cut.cell.epoch
            || snapshot
                .scopes
                .iter()
                .any(|(s, e)| cut.cell.scope_epochs.get(s).is_none_or(|p| e > p))
            || snapshot
                .block_ids
                .iter()
                .any(|id| !cut.cell.blocks.iter().any(|b| b.latched && &b.id == id))
        {
            return reject(Reject::ContinuityUnproven);
        }
        let specs: BTreeMap<_, _> = config
            .fact_specs
            .iter()
            .filter(|s| s.host == c.host)
            .map(|s| (&s.id, s))
            .collect();
        if snapshot
            .observations
            .iter()
            .map(|v| &v.source)
            .collect::<BTreeSet<_>>()
            != specs.keys().copied().collect()
            || base.source_sessions.keys().collect::<BTreeSet<_>>()
                != specs.keys().copied().collect()
        {
            return reject(Reject::ContinuityUnproven);
        }
        for observation in &snapshot.observations {
            let spec = specs[&observation.source];
            if base.source_sessions.get(&observation.source) != Some(&observation.generation)
                || observation.schema != spec.schema
                || observation.unit != spec.unit
            {
                return reject(Reject::ContinuityUnproven);
            }
            if !observation.quality_good
                || !observation.origin_age_bounded
                || observation.disputed
                || observation.uncertainty_ns > spec.maximum_uncertainty_ns
                || now
                    .age_ns(&observation.acquired_at)
                    .and_then(|n| n.checked_add(observation.uncertainty_ns.0))
                    .is_none_or(|n| n > spec.maximum_age_ns.0)
            {
                return reject(Reject::ConditionUnknown);
            }
        }
        let resources: BTreeSet<_> = config
            .steps
            .iter()
            .filter(|s| s.host == c.host)
            .flat_map(|s| s.intent.resource_set.iter())
            .collect();
        if snapshot.resource_fences.keys().collect::<BTreeSet<_>>() != resources {
            return reject(Reject::ContinuityUnproven);
        }
        for (resource, value) in &snapshot.resource_fences {
            let (_, maximum): (_, Counter) = load(
                tx,
                "host-link-fence",
                (&c.host, resource),
                "rx.internal.host-link-fence.v1",
            )?;
            if allowed.map_or(*value != maximum, |a| {
                a.get(resource).is_none_or(|v| !v.contains(value))
            }) {
                return reject(Reject::ContinuityUnproven);
            }
        }
        let known: Vec<_> = works
            .iter()
            .filter(|w| {
                w.host == c.host
                    && &w.cell == id
                    && w.operation.disposition() != rx_domain::operation::Disposition::Released
                    && w.host_journal == base.delivery_journal
            })
            .collect();
        if known.len() > crate::host_recovery::MAX_OPERATIONS
            || snapshot
                .pending_operations
                .iter()
                .any(|v| !known.iter().any(|w| w.operation.id() == v))
            || snapshot
                .pending_permits
                .iter()
                .any(|v| !known.iter().any(|w| &w.permit == v))
        {
            return reject(Reject::ContinuityUnproven);
        }
        host_recovery::retained_configuration(
            tx,
            crate::runtime_invalidation::configuration_digest(config)
                .map_err(StoreError::Integrity)?,
            base,
            observed,
            read.configuration.snapshot.binding_digest,
        )?;
    }
    Ok(())
}
