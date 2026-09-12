use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn submit(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: SubmitWork,
    ) -> Result<Work> {
        let command = SubmitWork {
            intent: command.intent.normalized().map_err(domain_error)?,
            ..command
        };
        let activation_id = &command.activation;
        let slot = &command.slot;
        let digest = command.intent.digest().map_err(domain_error)?;
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (activation_revision, mut activation): (_, Activation) =
                load(tx, "activationid", activation_id, ACTIVATION)?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &activation.run, RUN)?;
            if command.run != activation.run || command.part != activation.part {
                return reject(Reject::InvalidInput);
            }
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Executor,
                false,
            )?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Cell.SubmitOperation",
                request_key,
                &(&run.id, activation_id, slot, digest),
            )?;
            if let Some(work) = prior(tx, &scope, fingerprint, WORK)? {
                return Ok(work);
            }
            if let Some(existing) = activation.slots.get(slot) {
                let (_, work): (_, Work) = load(tx, "work", existing, WORK)?;
                if work.intent.digest().map_err(domain_error)? != digest {
                    return Err(StoreError::KeyConflict);
                }
                remember(tx, &scope, fingerprint, WORK, &work)?;
                return Ok(work);
            }
            let work = submit_transition(
                tx,
                DispatchState {
                    activation: &mut activation,
                    activation_revision,
                    run: &mut run,
                    run_revision,
                },
                &command,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )?;
            remember(tx, &scope, fingerprint, WORK, &work)?;
            Ok(work)
        })
    }
    pub fn inspect_work(&mut self, identity: &Identity, operation: &Id) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            authorize_read(tx, identity, meta, &now, &work.cell)?;
            Ok(work)
        })
    }
}

pub(super) struct DispatchState<'a> {
    pub activation: &'a mut Activation,
    pub activation_revision: Counter,
    pub run: &'a mut Run,
    pub run_revision: Counter,
}
pub(super) fn submit_transition(
    tx: &mut dyn Transaction,
    state: DispatchState<'_>,
    command: &SubmitWork,
    context: ProcessingContext<'_>,
) -> Result<Work> {
    let DispatchState {
        activation,
        activation_revision,
        run,
        run_revision,
    } = state;
    let ProcessingContext {
        identity,
        meta,
        now,
    } = context;
    let intent = &command.intent;
    let digest = intent.digest().map_err(domain_error)?;
    let slot = &command.slot;
    let expected_cell = command.expected_cell;
    let expected_run = command.expected_run;
    let (cell_revision, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
    check_revision(cell_revision, expected_cell)?;
    check_revision(run_revision, expected_run)?;
    active_run(tx, &cell, run, identity, meta, now)?;
    if cell.configuration.process.is_some() {
        super::process::eligible_node(tx, run, &cell, &activation.node, activation.visit)?;
    }
    validate_part(tx, run, &activation.part)?;
    let step = cell
        .configuration
        .steps
        .iter()
        .find(|s| s.id == activation.node)
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
    if slot.as_str() != "main" || step.intent.digest().map_err(domain_error)? != digest {
        return reject(Reject::CapabilityMissing);
    }
    predecessors_done(tx, run, &activation.part, step)?;
    let proof = evaluate(tx, &cell, &step.conditions, now)?;
    let host = prepared_host(tx, &cell, &step.host, meta, now)?;
    let expires = now
        .ticks_ns
        .0
        .checked_add(cell.configuration.permit_ttl_ns.0)
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let proof_until = proof
        .valid_until
        .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
    let expires = expires
        .min(host.grant.valid_until.ticks_ns.0)
        .min(proof_until.ticks_ns.0);
    if expires <= now.ticks_ns.0 {
        return reject(Reject::ConditionUnknown);
    }
    let operation_id = id();
    if run.purpose == Some(Purpose::Setup) {
        run.budget
            .as_mut()
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?
            .consume(Consumption::Operation(operation_id.clone()))
            .map_err(|_| StoreError::Rejected(Reject::BudgetExhausted))?;
    }
    for resource_id in &intent.resource_set {
        let key_ = key("resource", resource_id);
        let record = tx.get(&key_)?;
        let mut resource = if let Some(r) = &record {
            decode::<Resource>(r, RESOURCE)?
        } else {
            Resource {
                id: resource_id.clone(),
                fence: host.grant.fence,
                holder: None,
                quarantined: false,
            }
        };
        if resource.holder.is_some() || resource.quarantined {
            return reject(Reject::Busy);
        }
        if host.grant.fence < resource.fence {
            return reject(Reject::StaleEpoch);
        }
        resource.fence = host.grant.fence;
        resource.holder = Some(operation_id.clone());
        tx.put(
            &key_,
            record.map(|r| r.revision),
            &doc(RESOURCE, &resource)?,
        )?;
    }
    let qualification = cell
        .qualification
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::QualificationRequired))?;
    let permit = Permit {
        id: id(),
        operation: operation_id.clone(),
        intent_digest: digest,
        cell: run.cell.clone(),
        mandate: run
            .mandate
            .clone()
            .ok_or(StoreError::Rejected(Reject::MandateRevoked))?,
        epoch: cell.epoch,
        scopes: cell.scope_epochs.clone(),
        host: step.host.clone(),
        host_boot: host.boot_id,
        expires_at: TimePoint {
            clock_id: now.clock_id.clone(),
            ticks_ns: Counter(expires),
        },
        state: PermitState::Issued,
        grant: host.grant,
        qualification: qualification.id.clone(),
        evidence_ids: proof.evidence_ids,
        qualification_revision: qualification.revision,
        envelope_digest: cell.configuration.envelope.sha256,
        purpose: run
            .purpose
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
        issued_at: now.clone(),
        condition_ids: step.condition_ids.clone(),
        condition_revision: step.condition_revision,
    };
    let work = Work {
        operation: Operation::admitted(operation_id.clone(), digest),
        intent: command.intent.clone(),
        cell: run.cell.clone(),
        run: run.id.clone(),
        part: activation.part.clone(),
        activation: activation.id.clone(),
        slot: slot.clone(),
        host: step.host.clone(),
        permit: permit.id.clone(),
        invocation: None,
        completion: step.completion.clone(),
        host_journal: host.delivery_journal,
        handover_max_age_ns: step.handover_max_age_ns,
    };
    activation.slots.insert(slot.clone(), operation_id.clone());
    save(tx, "work", &operation_id, None, WORK, &work)?;
    save(tx, "permit", &permit.id, None, PERMIT, &permit)?;
    save(
        tx,
        "activationid",
        &activation.id,
        Some(activation_revision),
        ACTIVATION,
        activation,
    )?;
    let (mapping_revision, _): (_, Activation) = load(
        tx,
        "activation",
        (&run.id, &activation.node, activation.visit),
        ACTIVATION,
    )?;
    save(
        tx,
        "activation",
        (&run.id, &activation.node, activation.visit),
        Some(mapping_revision),
        ACTIVATION,
        activation,
    )?;
    save(tx, "run", &run.id, Some(run_revision), RUN, run)?;
    tx.enqueue(
        &operation_id,
        &doc(
            DELIVERY,
            &Delivery::Prepare {
                operation: operation_id.clone(),
                host: work.host.clone(),
                permit: permit.id,
            },
        )?,
    )?;
    event(tx, "rx.event.operation-admitted.v1", &work)?;
    Ok(work)
}
