use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn hold(&mut self, identity: &Identity, request_key: &str, cell_id: &Name) -> Result<Cell> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(cell_id),
                Role::Operator,
                false,
            )?;
            let (scope, fingerprint) =
                request(meta, &principal, "Cell.Hold", request_key, cell_id)?;
            if let Some(value) = prior(tx, &scope, fingerprint, CELL)? {
                return Ok(value);
            }
            invalidate_closure(tx, cell_id, BlockReason::OperatorHold)?;
            let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            remember(tx, &scope, fingerprint, CELL, &cell)?;
            Ok(cell)
        })
    }
}

pub(super) fn invalidate_cell(
    tx: &mut dyn Transaction,
    cell: &mut Cell,
    revision: Counter,
    reason: BlockReason,
) -> Result<()> {
    invalidate_cell_for_pause(tx, cell, revision, reason, None, None)
}
fn invalidate_cell_for_pause(
    tx: &mut dyn Transaction,
    cell: &mut Cell,
    revision: Counter,
    reason: BlockReason,
    paused_run: Option<&Id>,
    mut fence_ids: Option<&mut Vec<(Name, Id)>>,
) -> Result<()> {
    cell.epoch = cell.epoch.increment().map_err(domain_error)?;
    for epoch in cell.scope_epochs.values_mut() {
        *epoch = epoch.increment().map_err(domain_error)?;
    }
    if cell.commissioning == Some(Commissioning::Commissioned) {
        cell.commissioning = Some(Commissioning::RevalidationRequired);
    }
    cell.blocks.push(Block {
        id: id(),
        created_revision: Some(revision.increment().map_err(domain_error)?),
        case_id: None,
        reason,
        latched: true,
        scopes: cell.configuration.scopes.clone(),
    });
    for record in tx.scan("run/")? {
        let mut run: Run = decode(&record, RUN)?;
        if run.cell != cell.configuration.id {
            continue;
        }
        if matches!(
            run.state,
            RunState::Executing | RunState::Prepared | RunState::Paused
        ) {
            run.state = if paused_run == Some(&run.id) {
                RunState::Paused
            } else {
                RunState::RecoveryRequired
            };
            run.executor_session = None;
            run.pending_attempt = None;
            tx.put(&record.key, Some(record.revision), &doc(RUN, &run)?)?;
        }
    }
    for record in tx.scan("attempt/")? {
        let mut a: StartAttempt = decode(&record, ATTEMPT)?;
        if a.cell == cell.configuration.id
            && matches!(a.status, StartStatus::Pending | StartStatus::Arming)
        {
            a.status = StartStatus::Rejected;
            tx.put(&record.key, Some(record.revision), &doc(ATTEMPT, &a)?)?;
        }
    }
    for record in tx.scan("mandate/")? {
        let mut m: Mandate = decode(&record, MANDATE)?;
        if m.cell == cell.configuration.id && m.state == MandateState::Active {
            m.state = MandateState::Revoked;
            tx.put(&record.key, Some(record.revision), &doc(MANDATE, &m)?)?;
        }
    }
    for record in tx.scan("permit/")? {
        let mut p: Permit = decode(&record, PERMIT)?;
        if p.cell == cell.configuration.id && p.state == PermitState::Issued {
            p.state = PermitState::Voided;
            tx.put(&record.key, Some(record.revision), &doc(PERMIT, &p)?)?;
        }
    }
    for record in tx.scan("work/")? {
        let mut work: Work = decode(&record, WORK)?;
        if work.cell != cell.configuration.id {
            continue;
        }
        let mut changed = false;
        if work.operation.outcome() == Outcome::None {
            let voided = match tx.transition_outbox(
                work.operation.id(),
                rx_ports::OutboxState::New,
                rx_ports::OutboxState::Voided,
            ) {
                Ok(()) => true,
                Err(StoreError::OutboxConflict) => false,
                Err(e) => return Err(e),
            };
            let authorization = super::delivery::key_id(work.operation.id(), "authorize");
            let authorization_voided = if tx.outbox(&authorization)?.is_some() {
                match tx.transition_outbox(
                    &authorization,
                    rx_ports::OutboxState::New,
                    rx_ports::OutboxState::Voided,
                ) {
                    Ok(()) => true,
                    Err(StoreError::OutboxConflict) => false,
                    Err(error) => return Err(error),
                }
            } else {
                false
            };
            if (voided || authorization_voided)
                && work.operation.knowledge() == rx_domain::operation::Knowledge::NotSent
            {
                let evidence_id = id();
                tx.put(
                    &key("dispatchproof", &evidence_id),
                    None,
                    &doc(
                        "rx.evidence.void-before-dispatch.v1",
                        &(work.operation.id(), &evidence_id),
                    )?,
                )?;
                work.operation
                    .conclude(rx_domain::operation::Conclusion {
                        outcome: Outcome::NotExecuted,
                        evidence_ids: vec![evidence_id],
                    })
                    .map_err(domain_error)?;
            } else {
                work.operation.lose_continuity().map_err(domain_error)?;
            }
            changed = true;
        }
        for resource_id in &work.intent.resource_set {
            let (revision, mut resource): (_, Resource) =
                load(tx, "resource", resource_id, RESOURCE)?;
            if resource.holder.as_ref() == Some(work.operation.id()) {
                resource.quarantined = true;
                work.operation.quarantine().map_err(domain_error)?;
                changed = true;
                save(
                    tx,
                    "resource",
                    resource_id,
                    Some(revision),
                    RESOURCE,
                    &resource,
                )?;
            }
        }
        if changed {
            tx.put(&record.key, Some(record.revision), &doc(WORK, &work)?)?;
            event(tx, "rx.event.operation-invalidated.v1", &work)?;
        }
    }
    for host in &cell.configuration.hosts {
        let message = id();
        tx.enqueue(
            &message,
            &doc(
                DELIVERY,
                &Delivery::Fence {
                    cell: cell.configuration.id.clone(),
                    host: host.clone(),
                    epoch: cell.epoch,
                    scopes: cell.scope_epochs.clone(),
                    block_ids: cell
                        .blocks
                        .iter()
                        .filter(|b| b.latched)
                        .map(|b| b.id.clone())
                        .collect(),
                },
            )?,
        )?;
        if let Some(ids) = fence_ids.as_deref_mut() {
            ids.push((host.clone(), message));
        }
    }
    save(
        tx,
        "cell",
        &cell.configuration.id,
        Some(revision),
        CELL,
        cell,
    )?;
    event(tx, "rx.event.cell-invalidated.v1", cell)
}

pub(super) fn invalidate_closure(
    tx: &mut dyn Transaction,
    origin: &Name,
    reason: BlockReason,
) -> Result<BTreeSet<Name>> {
    invalidate_closure_for_pause(tx, origin, reason, None)
}
pub(super) fn pause_closure(
    tx: &mut dyn Transaction,
    origin: &Name,
    run: &Id,
) -> Result<BTreeSet<Name>> {
    invalidate_closure_for_pause(tx, origin, BlockReason::ExecutorPause, Some(run))
}
fn invalidate_closure_for_pause(
    tx: &mut dyn Transaction,
    origin: &Name,
    reason: BlockReason,
    paused_run: Option<&Id>,
) -> Result<BTreeSet<Name>> {
    let cells = tx
        .scan("cell/")?
        .into_iter()
        .map(|r| Ok((r.revision, decode::<Cell>(&r, CELL)?)))
        .collect::<Result<Vec<_>>>()?;
    let selected = affected_cells_from(&cells, origin)?;
    for (revision, mut cell) in cells {
        if selected.contains(&cell.configuration.id) {
            invalidate_cell_for_pause(tx, &mut cell, revision, reason, paused_run, None)?;
        }
    }
    Ok(selected)
}

pub(super) fn affected_cells(tx: &mut dyn Transaction, origin: &Name) -> Result<BTreeSet<Name>> {
    let cells = tx
        .scan("cell/")?
        .into_iter()
        .map(|r| Ok((r.revision, decode::<Cell>(&r, CELL)?)))
        .collect::<Result<Vec<_>>>()?;
    affected_cells_from(&cells, origin)
}
fn affected_cells_from(cells: &[(Counter, Cell)], origin: &Name) -> Result<BTreeSet<Name>> {
    let initial = cells
        .iter()
        .find(|(_, c)| &c.configuration.id == origin)
        .ok_or(StoreError::Rejected(Reject::NotFound))?;
    let mut selected = BTreeSet::from([initial.1.configuration.id.clone()]);
    loop {
        let scopes: BTreeSet<_> = cells
            .iter()
            .filter(|(_, c)| selected.contains(&c.configuration.id))
            .flat_map(|(_, c)| c.configuration.scopes.clone())
            .collect();
        let resources: BTreeSet<_> = cells
            .iter()
            .filter(|(_, c)| selected.contains(&c.configuration.id))
            .flat_map(|(_, c)| {
                c.configuration
                    .steps
                    .iter()
                    .flat_map(|s| s.intent.resource_set.clone())
            })
            .collect();
        let before = selected.len();
        for (_, cell) in cells {
            if cell.configuration.scopes.iter().any(|s| scopes.contains(s))
                || cell
                    .configuration
                    .steps
                    .iter()
                    .any(|s| s.intent.resource_set.iter().any(|r| resources.contains(r)))
            {
                selected.insert(cell.configuration.id.clone());
            }
        }
        if selected.len() == before {
            break;
        }
    }
    Ok(selected)
}

pub(super) fn invalidate_fact_dependents(
    tx: &mut dyn Transaction,
    fact: &FactRecord,
    reason: BlockReason,
) -> Result<()> {
    let cells = tx
        .scan("cell/")?
        .iter()
        .map(|r| decode::<Cell>(r, CELL))
        .collect::<Result<Vec<_>>>()?;
    let mut visited = BTreeSet::new();
    for cell in cells {
        if !visited.contains(&cell.configuration.id)
            && cell
                .configuration
                .fact_specs
                .iter()
                .any(|s| s.id == fact.id && s.host == fact.source_host)
        {
            visited.extend(invalidate_closure(tx, &cell.configuration.id, reason)?);
        }
    }
    Ok(())
}

pub(super) fn invalidate_for_change(
    tx: &mut dyn Transaction,
    cell: &mut Cell,
    revision: Counter,
) -> Result<Vec<(Name, Id)>> {
    let mut fences = Vec::new();
    invalidate_cell_for_pause(
        tx,
        cell,
        revision,
        BlockReason::ConfigurationChange,
        None,
        Some(&mut fences),
    )?;
    Ok(fences)
}
