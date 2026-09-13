use super::*;
use rx_ports::OutboxState;

fn producer(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    host: &Name,
) -> Result<(Counter, EvidenceProducer)> {
    let (revision, p): (_, EvidenceProducer) =
        load(tx, "producer", host, "rx.internal.evidence-producer.v1")?;
    authorize(
        tx,
        &Identity {
            principal: host.clone(),
            session: p.session.clone(),
            terminal: None,
        },
        meta,
        now,
        None,
        Role::Host,
        false,
    )?;
    if p.principal != *host {
        return reject(Reject::ContinuityUnproven);
    }
    Ok((revision, p))
}
fn transport(
    tx: &mut dyn Transaction,
    meta: &Installation,
    host: &Name,
) -> Result<baseline::TransportPin> {
    let (_, v): (_, TransportRegistration) = load(tx, "host-recovery-transport", host, TRANSPORT)?;
    if v.runtime_boot != meta.runtime_boot {
        return reject(Reject::CapabilityMissing);
    }
    v.pin
        .validate_current_bindings()
        .map_err(StoreError::Integrity)?;
    Ok(v.pin)
}
fn live_authority(tx: &mut dyn Transaction, cell: &Name) -> Result<bool> {
    for row in tx.scan("run/")? {
        let run: Run = decode(&row, RUN)?;
        if run.cell == *cell && matches!(run.state, RunState::Executing | RunState::Prepared) {
            return Ok(true);
        }
    }
    for row in tx.scan("mandate/")? {
        let mandate: Mandate = decode(&row, MANDATE)?;
        if mandate.cell == *cell && mandate.state == MandateState::Active {
            return Ok(true);
        }
    }
    for row in tx.scan("permit/")? {
        let permit: Permit = decode(&row, PERMIT)?;
        if permit.cell == *cell && permit.state == PermitState::Issued {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn build(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    host: &Name,
    origin: &Name,
) -> Result<r::Context> {
    lifecycle::require_serving(tx)?;
    let actor = authorize(
        tx,
        identity,
        meta,
        now,
        Some(origin),
        Role::ReleaseManager,
        true,
    )?;
    let impact = process_change::prospective_impact(
        tx,
        origin,
        &BTreeSet::from([host.clone()]),
        &BTreeSet::new(),
    )?;
    if impact.cells.iter().any(|c| !actor.cells.contains(&c.id)) {
        return reject(Reject::Forbidden);
    }
    let pin = transport(tx, meta, host)?;
    let (producer_revision, p) = producer(tx, meta, now, host)?;
    let mut host_cells = Vec::new();
    let mut cells = BTreeMap::new();
    for target in &impact.cells {
        let (revision, cell): (_, Cell) = load(tx, "cell", &target.id, CELL)?;
        if cell.configuration.hosts.contains(host) {
            host_cells.push(target.id.clone());
        }
        cells.insert(
            target.id.clone(),
            r::CellCut {
                revision,
                epoch: cell.epoch,
                scopes: cell.scope_epochs,
                configuration_digest: crate::runtime_invalidation::configuration_digest(
                    &cell.configuration,
                )
                .map_err(StoreError::Integrity)?,
                configuration: cell.configuration,
                blocks: cell.blocks,
                runtime_origins: BTreeMap::new(),
            },
        );
    }
    host_cells.sort();
    if host_cells.is_empty() || host_cells.len() > r::MAX_CELLS || !host_cells.contains(origin) {
        return reject(Reject::InvalidInput);
    }
    let mut blockers = Vec::new();
    let mut registrations = BTreeMap::new();
    let mut previous_runtime_boot = None;
    for cell in &host_cells {
        let Some(reg_row) = tx.get(&key("host", (cell, host)))? else {
            blockers.push(r::Blocker::RegistrationMissing { cell: cell.clone() });
            continue;
        };
        let reg: HostRegistration = decode(&reg_row, HOST)?;
        let (_, plan_id): (_, Id) = load(
            tx,
            "host-link-current",
            (host, cell),
            "rx.internal.host-link-id.v1",
        )?;
        let (plan_revision, plan): (_, crate::host_link::Plan) = load(
            tx,
            "host-link-plan",
            &plan_id,
            "rx.internal.host-link-plan.v1",
        )?;
        let Some(base) = baseline::load(tx, &plan_id)? else {
            blockers.push(r::Blocker::BaselineMissing { cell: cell.clone() });
            continue;
        };
        if previous_runtime_boot
            .as_ref()
            .is_some_and(|old| old != &base.runtime_boot)
        {
            blockers.push(r::Blocker::IdentityChanged { cell: cell.clone() });
        }
        previous_runtime_boot.get_or_insert(base.runtime_boot.clone());
        if base.installation != meta.id
            || base.store_generation != meta.store_generation
            || base.transport != pin
            || base.host != *host
            || base.cell != *cell
            || reg.id != *host
            || reg.cell != *cell
            || reg.boot_id != base.host_boot
            || reg.delivery_journal != base.delivery_journal
            || reg.source_sessions != base.source_sessions
            || reg.session != base.producer_session
            || p.authentication_binding != base.producer_authentication_binding
            || p.peer_boot != base.host_boot
            || p.journal != base.evidence_journal
            || p.session == base.producer_session
            || p.cells.get(cell) != Some(&cells[cell].configuration.definition.sha256)
            || !plan.bound
            || plan.host != *host
            || plan.cell != *cell
            || plan.host_boot != base.host_boot
            || plan.evidence_journal != base.evidence_journal
            || plan.delivery_journal != base.delivery_journal
            || plan.source_sessions != base.source_sessions
            || plan.producer_session != base.producer_session
        {
            blockers.push(r::Blocker::IdentityChanged { cell: cell.clone() });
        }
        registrations.insert(
            cell.clone(),
            r::RegistrationCut {
                revision: reg_row.revision,
                digest: digest(&reg)?,
                plan: plan_id,
                plan_revision,
                plan_digest: digest(&plan)?,
                baseline_digest: base.digest().map_err(StoreError::Integrity)?,
            },
        );
    }
    // Only a retained, explicitly completed prior recovery can advance the continuity anchor.
    let mut anchor = None;
    if let Some(row) = tx.get(&key("host-recovery-last", host))? {
        let old: Id = decode(&row, REFERENCE)?;
        let b = read(tx, &old)?;
        if b.phase == r::Phase::RecoveryOnly
            && b.context.runtime_boot != meta.runtime_boot
            && b.context.host_cells == host_cells
            && b.context.transport == pin
            && b.context.producer.peer_boot == p.peer_boot
            && b.context.producer.journal == p.journal
            && b.context.producer.authentication_binding == p.authentication_binding
            && registrations.iter().all(|(cell, reg)| {
                b.context
                    .registrations
                    .get(cell)
                    .is_some_and(|old| old.baseline_digest == reg.baseline_digest)
            })
        {
            previous_runtime_boot = Some(b.context.runtime_boot);
            anchor = Some(old);
        }
    }
    let previous_runtime_boot = previous_runtime_boot.unwrap_or_else(|| meta.runtime_boot.clone());
    for (cell_id, cut) in &mut cells {
        for block in cut
            .blocks
            .iter()
            .filter(|b| b.latched && b.reason == BlockReason::RuntimeRestart)
        {
            let Some(origin) =
                crate::runtime_invalidation::read_for_cell(tx, meta, cell_id, &block.id)?
            else {
                continue;
            };
            if origin.runtime_boot == meta.runtime_boot
                && origin.previous_runtime_boot == previous_runtime_boot
                && origin.after.configuration_digest == cut.configuration_digest
                && origin.after.scope_epochs.keys().collect::<BTreeSet<_>>()
                    == cut.scopes.keys().collect()
            {
                cut.runtime_origins.insert(
                    block.id.clone(),
                    origin.digest().map_err(StoreError::Integrity)?,
                );
            }
        }
        if cut.runtime_origins.is_empty() {
            blockers.push(r::Blocker::RestartOriginMissing {
                cell: cell_id.clone(),
            });
        }
        if live_authority(tx, cell_id)? {
            blockers.push(r::Blocker::LiveAuthority {
                cell: cell_id.clone(),
            });
        }
    }
    let operations = operations(tx, host, &host_cells)?;
    if operations.len() > r::MAX_OPERATIONS {
        blockers.push(r::Blocker::TooManyOperations);
    }
    Ok(r::Context {
        installation: meta.id.clone(),
        store_generation: meta.store_generation.clone(),
        runtime_boot: meta.runtime_boot.clone(),
        clock_id: meta.clock_id.clone(),
        host: host.clone(),
        origin: origin.clone(),
        transport: pin,
        producer: p,
        producer_revision,
        anchor,
        previous_runtime_boot,
        host_cells,
        cells,
        registrations,
        operations,
        blockers,
    })
}
fn operations(
    tx: &mut dyn Transaction,
    host: &Name,
    cells: &[Name],
) -> Result<BTreeMap<Id, r::OperationCut>> {
    let mut operations = BTreeMap::new();
    for row in tx.scan("work/")? {
        let work: Work = decode(&row, WORK)?;
        if work.host != *host
            || !cells.contains(&work.cell)
            || work.operation.disposition() == rx_domain::operation::Disposition::Released
        {
            continue;
        }
        let mut messages = Vec::new();
        for message in [
            work.operation.id().clone(),
            super::super::delivery::key_id(work.operation.id(), "authorize"),
        ] {
            if tx.outbox(&message)?.is_some_and(|o| {
                matches!(o.state, OutboxState::EmitEntered | OutboxState::Delivered)
            }) {
                messages.push(message);
            }
        }
        if messages.is_empty() {
            continue;
        }
        operations.insert(
            work.operation.id().clone(),
            r::OperationCut {
                operation: work.operation.id().clone(),
                cell: work.cell,
                permit: work.permit,
                intent_digest: work.intent.digest().map_err(domain_error)?,
                profile_digest: work.intent.profile_digest,
                host_journal: work.host_journal,
                invocation: work.invocation,
                messages,
            },
        );
    }
    Ok(operations)
}
pub(super) fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    c: &r::Context,
) -> Result<()> {
    lifecycle::require_serving(tx)?;
    if !c.blockers.is_empty()
        || c.installation != meta.id
        || c.store_generation != meta.store_generation
        || c.runtime_boot != meta.runtime_boot
        || c.clock_id != meta.clock_id
        || now.clock_id != c.clock_id
        || transport(tx, meta, &c.host)? != c.transport
    {
        return reject(Reject::ContinuityUnproven);
    }
    let (revision, p) = producer(tx, meta, now, &c.host)?;
    if revision != c.producer_revision || !same(&p, &c.producer)? {
        return reject(Reject::StaleRevision);
    }
    let impact = process_change::prospective_impact(
        tx,
        &c.origin,
        &BTreeSet::from([c.host.clone()]),
        &BTreeSet::new(),
    )?;
    if impact.cells.iter().map(|c| &c.id).collect::<BTreeSet<_>>() != c.cells.keys().collect() {
        return reject(Reject::StaleRevision);
    }
    for (cell_id, cut) in &c.cells {
        let (revision, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
        if revision != cut.revision
            || cell.epoch != cut.epoch
            || cell.scope_epochs != cut.scopes
            || !same(&cell.configuration, &cut.configuration)?
            || !same(&cell.blocks, &cut.blocks)?
            || live_authority(tx, cell_id)?
        {
            return reject(Reject::StaleRevision);
        }
        for (block, expected) in &cut.runtime_origins {
            let origin = crate::runtime_invalidation::read_for_cell(tx, meta, cell_id, block)?
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            if origin.digest().map_err(StoreError::Integrity)? != *expected {
                return reject(Reject::ContinuityUnproven);
            }
        }
    }
    for (cell_id, expected) in &c.registrations {
        let (revision, reg): (_, HostRegistration) = load(tx, "host", (cell_id, &c.host), HOST)?;
        let (plan_revision, plan): (_, crate::host_link::Plan) = load(
            tx,
            "host-link-plan",
            &expected.plan,
            "rx.internal.host-link-plan.v1",
        )?;
        let (_, current_plan): (_, Id) = load(
            tx,
            "host-link-current",
            (&c.host, cell_id),
            "rx.internal.host-link-id.v1",
        )?;
        let base = baseline::load(tx, &expected.plan)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if revision != expected.revision
            || digest(&reg)? != expected.digest
            || current_plan != expected.plan
            || plan_revision != expected.plan_revision
            || digest(&plan)? != expected.plan_digest
            || base.digest().map_err(StoreError::Integrity)? != expected.baseline_digest
        {
            return reject(Reject::StaleRevision);
        }
    }
    Ok(())
}
