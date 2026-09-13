use super::*;
use crate::host_binding_baseline::{
    self as data, BaselineSource, HostBindingBaseline, SourceReference,
};

fn same<T: Serialize>(left: &T, right: &T) -> Result<bool> {
    Ok(canonical::bytes(left).map_err(domain_error)?
        == canonical::bytes(right).map_err(domain_error)?)
}

pub(super) fn validate_provenance(
    provenance: &BootstrapProvenance,
    snapshot: &rx_domain::host_snapshot::HostSnapshot,
    read_started: &TimePoint,
    configuration: &CellConfiguration,
    now: &TimePoint,
) -> Result<()> {
    provenance
        .transport
        .validate_current_bindings()
        .map_err(StoreError::Invalid)?;
    provenance
        .configuration
        .validate()
        .map_err(StoreError::Invalid)?;
    provenance.host_read.validate().map_err(domain_error)?;
    let observed = &provenance.configuration.snapshot;
    let Some(cell) = observed
        .cells
        .iter()
        .find(|cell| cell.cell == snapshot.cell)
    else {
        return reject(Reject::ContinuityUnproven);
    };
    let configuration_digest = crate::runtime_invalidation::configuration_digest(configuration)
        .map_err(StoreError::Integrity)?;
    if !same(&provenance.host_read, snapshot)?
        || observed.host != snapshot.host
        || observed.host_boot != snapshot.host_boot
        || observed.delivery_journal != snapshot.delivery_journal
        || cell.cell != configuration.id
        || cell.definition != snapshot.definition
        || cell.envelope != snapshot.envelope
        || cell.environment != snapshot.environment
        || cell.epoch != snapshot.epoch
        || cell.scopes != snapshot.scopes
        || cell.blocked.len() != cell.blocked.iter().collect::<BTreeSet<_>>().len()
        || cell.blocked.iter().collect::<BTreeSet<_>>() != snapshot.block_ids.iter().collect()
        || cell.applied.as_ref().is_some_and(|applied| {
            applied.binding_digest != observed.binding_digest
                || applied.configuration != configuration_digest
        })
        || provenance
            .configuration_read_finished
            .age_ns(&provenance.configuration_read_started)
            .is_none_or(|age| age > 100_000_000)
        || now
            .age_ns(&provenance.configuration_read_started)
            .is_none_or(|age| age > 100_000_000)
        || read_started
            .age_ns(&provenance.configuration_read_finished)
            .is_none()
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}

/// Fresh reads may have new times/source evidence. They may not rewrite the original
/// transport/configuration identity of a reusable plan, including an already-bound plan.
pub(super) fn check_reuse(plan: &Plan, incoming: Option<&BootstrapProvenance>) -> Result<()> {
    reject_downgrade(plan, incoming)?;
    if let (Some(original), Some(incoming)) = (&plan.provenance, incoming) {
        let original_cell = original
            .configuration
            .snapshot
            .cells
            .iter()
            .find(|c| c.cell == plan.cell)
            .ok_or_else(|| StoreError::Integrity("plan configuration cell missing".into()))?;
        let incoming_cell = incoming
            .configuration
            .snapshot
            .cells
            .iter()
            .find(|c| c.cell == plan.cell)
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if original.transport != incoming.transport
            || original.configuration.snapshot.binding_digest
                != incoming.configuration.snapshot.binding_digest
            || !same(&original_cell.applied, &incoming_cell.applied)?
        {
            return Err(StoreError::KeyConflict);
        }
    }
    // In particular, Some provenance cannot be attached retroactively to a legacy None plan.
    Ok(())
}

pub(super) fn reject_downgrade(plan: &Plan, incoming: Option<&BootstrapProvenance>) -> Result<()> {
    if plan.provenance.is_some() && incoming.is_none() {
        return Err(StoreError::KeyConflict);
    }
    Ok(())
}

pub(super) fn create(
    tx: &mut dyn Transaction,
    meta: &Installation,
    producer: &EvidenceProducer,
    configuration: &CellConfiguration,
    plan: &Plan,
    registration: &HostRegistration,
    now: &TimePoint,
) -> Result<()> {
    let Some(provenance) = &plan.provenance else {
        return Ok(());
    };
    // The original pre-Fence observations need not equal the later bound epoch/scopes.
    // They were checked against each other and P's existing <= guards at preparation.
    validate_provenance(
        provenance,
        &provenance.host_read,
        &provenance.host_read.captured_at,
        configuration,
        &plan.prepared_at,
    )?;
    if !plan.bound
        || provenance.host_read.host != plan.host
        || provenance.host_read.cell != plan.cell
        || provenance.host_read.host_boot != plan.host_boot
        || provenance.host_read.delivery_journal != plan.delivery_journal
        || provenance.host_read.evidence_journal != plan.evidence_journal
        || provenance.host_read.definition != plan.definition
        || producer.session != plan.producer_session
        || producer.peer_boot != plan.host_boot
        || producer.journal != plan.evidence_journal
        || registration.id != plan.host
        || registration.cell != plan.cell
        || registration.boot_id != plan.host_boot
        || registration.session != plan.producer_session
        || registration.delivery_journal != plan.delivery_journal
        || registration.epoch != plan.epoch
        || registration.scopes != plan.scopes
        || registration.source_sessions != plan.source_sessions
    {
        return reject(Reject::ContinuityUnproven);
    }
    let context = provenance
        .configuration
        .snapshot
        .cells
        .iter()
        .find(|cell| cell.cell == plan.cell)
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    let mut source_references = BTreeMap::new();
    for source in &provenance.host_read.observations {
        if plan.source_sessions.get(&source.source) != Some(&source.generation) {
            return reject(Reject::ContinuityUnproven);
        }
        source_references.insert(
            source.source.clone(),
            SourceReference {
                generation: source.generation.clone(),
                evidence: source.evidence_id.clone(),
                observation: data::reference("rx.host-source-observation.v1", source)
                    .map_err(StoreError::Invalid)?,
            },
        );
    }
    let (_, receipt): (_, BoundReceipt) = load(
        tx,
        "host-link-receipt",
        &plan.id,
        "rx.internal.host-link-receipt.v1",
    )?;
    let source = BaselineSource {
        installation: meta.clone(),
        producer: producer.clone(),
        configuration: configuration.clone(),
        plan: plan.clone(),
        receipt,
        bound_at: now.clone(),
    };
    let value = HostBindingBaseline {
        schema: name(data::SCHEMA),
        plan: plan.id.clone(),
        source: data::reference(data::SOURCE_SCHEMA, &source).map_err(StoreError::Invalid)?,
        original_plan_digest: canonical::digest("RX-HOST-LINK-BOUND-PLAN-v1", plan)
            .map_err(domain_error)?,
        original_registration_digest: canonical::digest(
            "RX-HOST-LINK-REGISTRATION-v1",
            registration,
        )
        .map_err(domain_error)?,
        installation: meta.id.clone(),
        store_generation: meta.store_generation.clone(),
        runtime_boot: meta.runtime_boot.clone(),
        host: plan.host.clone(),
        cell: plan.cell.clone(),
        producer_authentication_binding: producer.authentication_binding,
        producer_session: plan.producer_session.clone(),
        platform_session: plan.platform_session.clone(),
        host_boot: plan.host_boot.clone(),
        evidence_journal: plan.evidence_journal.clone(),
        delivery_journal: plan.delivery_journal.clone(),
        source_sessions: plan.source_sessions.clone(),
        transport: provenance.transport.clone(),
        transport_digest: provenance.transport.digest().map_err(StoreError::Invalid)?,
        original_configuration_digest: crate::runtime_invalidation::configuration_digest(
            configuration,
        )
        .map_err(StoreError::Integrity)?,
        host_binding_digest: provenance.configuration.snapshot.binding_digest,
        applied_context: context.applied.clone(),
        epoch: plan.epoch,
        scopes: plan.scopes.clone(),
        host_read: data::reference("rx.host-snapshot.v1", &provenance.host_read)
            .map_err(StoreError::Invalid)?,
        host_read_captured_at: provenance.host_read.captured_at.clone(),
        configuration_read: data::reference(
            "rx.host-process-configuration-observation.v1",
            &provenance.configuration,
        )
        .map_err(StoreError::Invalid)?,
        configuration_read_started: provenance.configuration_read_started.clone(),
        configuration_read_finished: provenance.configuration_read_finished.clone(),
        source_references,
        bound_at: now.clone(),
    };
    value.validate().map_err(StoreError::Integrity)?;
    value
        .verify_source(&source)
        .map_err(StoreError::Integrity)?;
    let key = key(data::PREFIX, &plan.id);
    if tx.get(&key)?.is_some()
        || tx
            .get(&crate::persistence::key(data::SOURCE_PREFIX, &plan.id))?
            .is_some()
    {
        // A new bind must be the unique creator; exact commit replay returns its cached
        // receipt before entering here and never creates or repairs a missing baseline.
        return Err(StoreError::KeyConflict);
    }
    save(
        tx,
        data::SOURCE_PREFIX,
        &plan.id,
        None,
        data::SOURCE_SCHEMA,
        &source,
    )?;
    save(tx, data::PREFIX, &plan.id, None, data::SCHEMA, &value)?;
    Ok(())
}
