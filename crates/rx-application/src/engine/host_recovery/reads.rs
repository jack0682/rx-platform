use super::*;
use rx_domain::host_configuration as h;

fn window(now: &TimePoint, started: &TimePoint, finished: &TimePoint) -> Result<()> {
    if finished
        .age_ns(started)
        .is_none_or(|age| age > r::READ_WINDOW_NS)
        || now.age_ns(started).is_none_or(|age| age > r::READ_AGE_NS)
    {
        return reject(Reject::ConditionUnknown);
    }
    Ok(())
}
fn configuration(
    tx: &mut dyn Transaction,
    cut: &r::CellCut,
    base: &baseline::HostBindingBaseline,
    observed: &h::CellObservation,
    binding_digest: Digest,
) -> Result<()> {
    let Some(applied) = &observed.applied else {
        if base.applied_context.is_none()
            && cut.configuration_digest == base.original_configuration_digest
            && binding_digest == base.host_binding_digest
        {
            return Ok(());
        }
        return reject(Reject::ContinuityUnproven);
    };
    if applied.cell != base.cell
        || applied.configuration != cut.configuration_digest
        || applied.binding_digest != binding_digest
        || binding_digest != base.host_binding_digest
    {
        return reject(Reject::ContinuityUnproven);
    }
    let (_, stored): (_, crate::process_change::Change) =
        load(tx, "processchange", &applied.change, "rx.process-change.v1")?;
    let change = process_change::change(tx, &stored.id, &stored.cell)?;
    if !matches!(
        change.state,
        crate::process_change::State::AppliedUnqualified
            | crate::process_change::State::QualifiedActive
    ) {
        return reject(Reject::ContinuityUnproven);
    }
    let application = change
        .application
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    if !application
        .cells
        .iter()
        .any(|c| c.cell == base.cell && c.after.sha256 == cut.configuration_digest)
    {
        return reject(Reject::ContinuityUnproven);
    }
    let proof = application
        .host_proofs
        .iter()
        .find(|p| p.host == base.host && p.task == applied.request)
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    let (_, task) = configuration_dispatch::read(tx, &proof.task)?;
    let receipt = task
        .receipt
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    let request = task
        .request
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    if task.integrity_disputed
        || task.host != base.host
        || task.host_boot != base.host_boot
        || task.delivery_journal != base.delivery_journal
        || task.change != change.id
        || receipt.status != h::Status::AppliedUnqualified
        || receipt.host_boot != base.host_boot
        || receipt.journal != base.delivery_journal
        || receipt.sequence != applied.receipt_sequence
        || receipt.request_digest != proof.request_digest
        || request.id != applied.request
        || request.binding_digest != binding_digest
        || request.change != applied.change
        || canonical::digest("RX-HOST-CONFIGURATION-RECEIPT-v1", receipt).map_err(domain_error)?
            != proof.receipt_digest
        || !request.cells.iter().any(|c| {
            c.cell == base.cell
                && c.after_configuration == cut.configuration_digest
                && c.definition == observed.definition
                && c.envelope == observed.envelope
                && c.environment == observed.environment
        })
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}
pub(super) fn validate(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    c: &r::Context,
    read: &r::ReadEvidence,
    post_fence: bool,
) -> Result<()> {
    read.transport
        .validate_current_bindings()
        .map_err(StoreError::Invalid)?;
    read.configuration.validate().map_err(StoreError::Invalid)?;
    if read.transport != c.transport
        || read.cells.keys().collect::<BTreeSet<_>>() != c.host_cells.iter().collect()
        || read
            .configuration
            .snapshot
            .cells
            .iter()
            .map(|c| &c.cell)
            .collect::<BTreeSet<_>>()
            != c.host_cells.iter().collect()
        || read.configuration.snapshot.host != c.host
    {
        return reject(Reject::ContinuityUnproven);
    }
    window(
        now,
        &read.configuration_started,
        &read.configuration_finished,
    )?;
    for cell in &c.host_cells {
        let base = baseline::load(tx, &c.registrations[cell].plan)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        let cut = &c.cells[cell];
        let actual = &read.cells[cell];
        actual.snapshot.validate().map_err(domain_error)?;
        window(now, &actual.started, &actual.finished)?;
        let snapshot = &actual.snapshot;
        let configuration_observation = read
            .configuration
            .snapshot
            .cells
            .iter()
            .find(|o| &o.cell == cell)
            .unwrap();
        let expected_environment = match cut.configuration.environment {
            Environment::Simulation => "SIMULATION",
            Environment::Physical => "PHYSICAL",
        };
        if actual
            .started
            .age_ns(&read.configuration_finished)
            .is_none()
            || snapshot.captured_at.age_ns(&actual.started).is_none()
            || actual.finished.age_ns(&snapshot.captured_at).is_none()
            || snapshot.host != c.host
            || snapshot.host_boot != base.host_boot
            || snapshot.evidence_journal != base.evidence_journal
            || snapshot.delivery_journal != base.delivery_journal
            || snapshot.definition != cut.configuration.definition.sha256
            || snapshot.envelope != cut.configuration.envelope.sha256
            || snapshot.environment.as_str() != expected_environment
            || !snapshot.sources_available
            || read.configuration.snapshot.host_boot != base.host_boot
            || read.configuration.snapshot.delivery_journal != base.delivery_journal
            || configuration_observation.definition != snapshot.definition
            || configuration_observation.envelope != snapshot.envelope
            || configuration_observation.environment != snapshot.environment
            || configuration_observation.scopes != snapshot.scopes
            || configuration_observation.epoch != snapshot.epoch
            || configuration_observation
                .blocked
                .iter()
                .collect::<BTreeSet<_>>()
                != snapshot.block_ids.iter().collect()
        {
            return reject(Reject::ContinuityUnproven);
        }
        if snapshot.scopes.keys().collect::<BTreeSet<_>>() != cut.scopes.keys().collect()
            || snapshot.epoch > cut.epoch
            || snapshot
                .scopes
                .iter()
                .any(|(scope, epoch)| cut.scopes.get(scope).is_none_or(|current| epoch > current))
            || snapshot
                .block_ids
                .iter()
                .any(|id| !cut.blocks.iter().any(|b| b.latched && b.id == *id))
        {
            return reject(Reject::ContinuityUnproven);
        }
        if post_fence
            && (snapshot.epoch != cut.epoch
                || snapshot.scopes != cut.scopes
                || snapshot.block_ids.iter().collect::<BTreeSet<_>>()
                    != cut
                        .blocks
                        .iter()
                        .filter(|b| b.latched)
                        .map(|b| &b.id)
                        .collect())
        {
            return reject(Reject::HostNotPrepared);
        }
        let specs: BTreeMap<_, _> = cut
            .configuration
            .fact_specs
            .iter()
            .filter(|s| s.host == c.host)
            .map(|s| (&s.id, s))
            .collect();
        if snapshot
            .observations
            .iter()
            .map(|s| &s.source)
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
            if observation.uncertainty_ns > spec.maximum_uncertainty_ns
                || now
                    .age_ns(&observation.acquired_at)
                    .and_then(|age| age.checked_add(observation.uncertainty_ns.0))
                    .is_none_or(|age| age > spec.maximum_age_ns.0)
            {
                return reject(Reject::ConditionUnknown);
            }
        }
        let expected_resources: BTreeSet<_> = cut
            .configuration
            .steps
            .iter()
            .filter(|s| s.host == c.host)
            .flat_map(|s| s.intent.resource_set.iter())
            .collect();
        if snapshot.resource_fences.keys().collect::<BTreeSet<_>>() != expected_resources {
            return reject(Reject::ContinuityUnproven);
        }
        for (resource, observed) in &snapshot.resource_fences {
            let (_, maximum): (_, Counter) = load(
                tx,
                "host-link-fence",
                (&c.host, resource),
                "rx.internal.host-link-fence.v1",
            )?;
            if *observed != maximum {
                return reject(Reject::ContinuityUnproven);
            }
        }
        let operations: BTreeSet<_> = c
            .operations
            .values()
            .filter(|o| &o.cell == cell)
            .map(|o| &o.operation)
            .collect();
        let permits: BTreeSet<_> = c
            .operations
            .values()
            .filter(|o| &o.cell == cell)
            .map(|o| &o.permit)
            .collect();
        if snapshot
            .pending_operations
            .iter()
            .any(|id| !operations.contains(id))
            || snapshot
                .pending_permits
                .iter()
                .any(|id| !permits.contains(id))
        {
            return reject(Reject::ContinuityUnproven);
        }
        configuration(
            tx,
            cut,
            &base,
            configuration_observation,
            read.configuration.snapshot.binding_digest,
        )?;
    }
    Ok(())
}

/// Preserve actual source values and clocks through the existing fact validator. The old
/// registration is an immutable identity/source reference, never synthesized with a new session.
pub(super) fn ingest(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    c: &r::Context,
    read: &r::ReadEvidence,
) -> Result<()> {
    for cell_id in &c.host_cells {
        let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
        let (_, host): (_, HostRegistration) = load(tx, "host", (cell_id, &c.host), HOST)?;
        let mut facts = Vec::new();
        for source in &read.cells[cell_id].snapshot.observations {
            let spec = cell
                .configuration
                .fact_specs
                .iter()
                .find(|s| s.id == source.source && s.host == c.host)
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            facts.push(FactRecord {
                cell: cell_id.clone(),
                id: source.source.clone(),
                source_host: c.host.clone(),
                source_generation: source.generation.clone(),
                schema: source.schema.clone(),
                unit: source.unit.clone(),
                acquired_at: source.acquired_at.clone(),
                maximum_age_ns: spec.maximum_age_ns,
                acquisition_uncertainty_ns: source.uncertainty_ns,
                quality_good: source.quality_good,
                origin_age_bounded: source.origin_age_bounded,
                disputed: source.disputed,
                value: source.value.clone(),
                evidence_id: source.evidence_id.clone(),
            });
        }
        if !facts.is_empty() {
            super::super::observation::accept_facts(tx, &cell, &host, facts, now)?;
        }
    }
    Ok(())
}
