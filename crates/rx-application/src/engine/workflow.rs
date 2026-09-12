use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn create_run(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: CreateRun,
    ) -> Result<Run> {
        let cell_id = &command.cell;
        let expected_cell = command.expected_cell;
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(cell_id),
                Role::Operator,
                false,
            )?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "Workflow.CreateRun",
                request_key,
                &(cell_id, command.recipe_digest, command.site_config_digest),
            )?;
            if let Some(run) = prior(tx, &scope, fingerprint, RUN)? {
                return Ok(run);
            }
            lifecycle::require_serving(tx)?;
            let (revision, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            check_revision(revision, expected_cell)?;
            if command.recipe_digest != cell.configuration.recipe.sha256
                || command.site_config_digest != cell.configuration.site_config_digest
            {
                return reject(Reject::CapabilityMissing);
            }
            let run = Run {
                id: id(),
                cell: cell_id.clone(),
                recipe_digest: cell.configuration.recipe.sha256,
                envelope_digest: cell.configuration.envelope.sha256,
                purpose: None,
                state: RunState::Prepared,
                budget: None,
                executor_session: None,
                mandate: None,
                part_ids: vec![],
                pending_attempt: None,
            };
            run_configuration::create(tx, &run, &cell.configuration)?;
            save(tx, "run", &run.id, None, RUN, &run)?;
            remember(tx, &scope, fingerprint, RUN, &run)?;
            event(tx, "rx.event.run-created.v1", &run)?;
            Ok(run)
        })
    }
    pub fn start_run(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: StartRun,
    ) -> Result<StartAttempt> {
        let run_id = &command.run;
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Operator,
                true,
            )?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Cell.StartRun",
                request_key,
                &(
                    run_id,
                    command.envelope_digest,
                    command.purpose,
                    command.budget_unit,
                    command.budget_limit,
                ),
            )?;
            if let Some(attempt) = prior::<StartAttempt>(tx, &scope, fingerprint, ATTEMPT)? {
                return Ok(load::<StartAttempt>(tx, "attempt", &attempt.id, ATTEMPT)?.1);
            }
            let (run_revision, mut run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            let validated = operator_start::validate_candidate(
                tx,
                identity,
                meta,
                &now,
                operator_start::StartBasis {
                    run_revision,
                    run: &run,
                    cell_revision,
                    cell: &cell,
                },
                &command,
            )?;
            if run.budget.is_none() {
                run.budget = Some(
                    RunBudget::new(command.budget_unit, command.budget_limit)
                        .map_err(domain_error)?,
                );
                run.purpose = Some(command.purpose);
            }
            let attempt = StartAttempt {
                id: id(),
                run: run_id.clone(),
                cell: run.cell.clone(),
                expected_cell_revision: cell_revision,
                expected_run_revision: validated.next_run_revision,
                epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                host_boots: validated.host_boots,
                acknowledgments: BTreeMap::new(),
                executor_session: validated.executor_session,
                actor: principal.id,
                status: StartStatus::Arming,
                mandate: None,
                valid_until: validated.valid_until,
                terminal: identity.terminal.clone(),
            };
            run.pending_attempt = Some(attempt.id.clone());
            save(tx, "run", &run.id, Some(run_revision), RUN, &run)?;
            let approved_clear = validated.clear_blocks;
            for host in &cell.configuration.hosts {
                tx.enqueue(
                    &id(),
                    &doc(
                        DELIVERY,
                        &Delivery::Arm {
                            clear_blocks: approved_clear.clone(),
                            attempt: attempt.id.clone(),
                            host: host.clone(),
                            epoch: cell.epoch,
                            scopes: cell.scope_epochs.clone(),
                        },
                    )?,
                )?;
            }
            save(tx, "attempt", &attempt.id, None, ATTEMPT, &attempt)?;
            remember(tx, &scope, fingerprint, ATTEMPT, &attempt)?;
            event(tx, "rx.event.start-requested.v1", &attempt)?;
            Ok(attempt)
        })
    }
    pub fn acknowledge_arm(
        &mut self,
        identity: &Identity,
        ack: ArmAcknowledgment,
    ) -> Result<StartAttempt> {
        self.acknowledge_arm_delivery(identity, ack, None)
    }
    pub fn finish_arm_delivery(
        &mut self,
        identity: &Identity,
        message: &Id,
        ack: ArmAcknowledgment,
    ) -> Result<StartAttempt> {
        self.acknowledge_arm_delivery(identity, ack, Some(message))
    }
    fn acknowledge_arm_delivery(
        &mut self,
        identity: &Identity,
        ack: ArmAcknowledgment,
        message: Option<&Id>,
    ) -> Result<StartAttempt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, mut attempt): (_, StartAttempt) =
                load(tx, "attempt", &ack.attempt, ATTEMPT)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&attempt.cell),
                Role::Host,
                false,
            )?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&attempt.cell, &principal.id), HOST)?;
            if attempt.host_boots.get(&principal.id) != Some(&ack.host_boot)
                || ack.host_boot != host.boot_id
                || ack.delivery_journal != host.delivery_journal
                || ack.sequence.0 == 0
                || ack.epoch != attempt.epoch
                || ack.scopes != attempt.scopes
            {
                return reject(Reject::StaleEpoch);
            }
            if attempt.status == StartStatus::Started {
                finish_arm_outbox(tx, message, &principal.id, &attempt.id)?;
                return Ok(attempt);
            }
            if attempt.status == StartStatus::Rejected {
                return reject(Reject::StaleRevision);
            }
            let (cell_revision, mut cell): (_, Cell) = load(tx, "cell", &attempt.cell, CELL)?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &attempt.run, RUN)?;
            if cell_revision != attempt.expected_cell_revision
                || run_revision != attempt.expected_run_revision
                || run.state != RunState::Prepared
                || run.pending_attempt.as_ref() != Some(&attempt.id)
                || now.clock_id != attempt.valid_until.clock_id
                || now.ticks_ns >= attempt.valid_until.ticks_ns
            {
                attempt.status = StartStatus::Rejected;
                if run.pending_attempt.as_ref() == Some(&attempt.id) {
                    run.pending_attempt = None;
                    save(tx, "run", &run.id, Some(run_revision), RUN, &run)?;
                }
            } else {
                let (_, origin): (_, Principal) = load(tx, "principal", &attempt.actor, PRINCIPAL)?;
                if !origin.active
                    || !origin.roles.contains(&Role::Operator)
                    || !origin.cells.contains(&run.cell)
                {
                    return reject(Reject::Forbidden);
                }
                let (terminal_id, certificate) = attempt
                    .terminal
                    .as_ref()
                    .ok_or(StoreError::Rejected(Reject::Forbidden))?;
                let (_, terminal): (_, Terminal) = load(tx, "terminal", terminal_id, TERMINAL)?;
                if !terminal.active
                    || terminal.certificate_digest != *certificate
                    || !terminal.cells.contains(&run.cell)
                {
                    return reject(Reject::Forbidden);
                }
                ready(tx, &cell, &now)?;
                evaluate(tx, &cell, &cell.configuration.start_conditions, &now)?;
                let executor = current_session(
                    tx,
                    &cell.configuration.executor,
                    meta,
                    &now,
                    Role::Executor,
                    &cell.configuration.id,
                )?;
                if executor.id != attempt.executor_session {
                    return reject(Reject::Unauthenticated);
                }
                for h in &cell.configuration.hosts {
                    let current = prepared_host(tx, &cell, h, meta, &now)?;
                    if attempt.host_boots.get(h) != Some(&current.boot_id) {
                        return reject(Reject::HostNotPrepared);
                    }
                }
                attempt
                    .acknowledgments
                    .insert(principal.id.clone(), ack.host_boot.clone());
                if attempt.acknowledgments.len() == attempt.host_boots.len() {
                    let mandate = Mandate {
                        id: id(),
                        run: run.id.clone(),
                        cell: run.cell.clone(),
                        epoch: cell.epoch,
                        scopes: cell.scope_epochs.clone(),
                        executor_session: executor.id.clone(),
                        state: MandateState::Active,
                        actor: attempt.actor.clone(),
                        attempt: attempt.id.clone(),
                    };
                    cell.mode = Some(if run.purpose == Some(Purpose::Production) {
                        OperatingMode::Automatic
                    } else {
                        OperatingMode::Setup
                    });
                    save(tx, "cell", &attempt.cell, Some(cell_revision), CELL, &cell)?;
                    run.state = RunState::Executing;
                    run.executor_session = Some(executor.id);
                    run.mandate = Some(mandate.id.clone());
                    run.pending_attempt = None;
                    attempt.status = StartStatus::Started;
                    attempt.mandate = Some(mandate.id.clone());
                    save(tx, "mandate", &mandate.id, None, MANDATE, &mandate)?;
                    save(tx, "run", &run.id, Some(run_revision), RUN, &run)?;
                    event(tx, "rx.event.mandate-created.v1", &mandate)?;
                }
            }
            save(
                tx,
                "attempt",
                &attempt.id,
                Some(revision),
                ATTEMPT,
                &attempt,
            )?;
            event(tx, "rx.event.start-changed.v1", &attempt)?;
            finish_arm_outbox(tx, message, &principal.id, &attempt.id)?;
            Ok(attempt)
        })
    }
    pub fn inspect_run(&mut self, identity: &Identity, run_id: &Id) -> Result<(Counter, Run)> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let result: (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize_read(tx, identity, meta, &now, &result.1.cell)?;
            Ok(result)
        })
    }
    pub fn inspect_attempt(
        &mut self,
        identity: &Identity,
        attempt_id: &Id,
    ) -> Result<StartAttempt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, attempt): (_, StartAttempt) = load(tx, "attempt", attempt_id, ATTEMPT)?;
            authorize_read(tx, identity, meta, &now, &attempt.cell)?;
            Ok(attempt)
        })
    }
    pub fn begin_part(
        &mut self,
        identity: &Identity,
        request_key: &str,
        run_id: &Id,
        expected_budget: Counter,
    ) -> Result<PartAttempt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, mut run): (_, Run) = load(tx, "run", run_id, RUN)?;
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
                "Cell.BeginPartAttempt",
                request_key,
                run_id,
            )?;
            if let Some(part) = prior(tx, &scope, fingerprint, PART)? {
                return Ok(part);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            let part = begin_part_transition(
                tx,
                &cell,
                &mut run,
                revision,
                expected_budget,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )?
            .part;
            remember(tx, &scope, fingerprint, PART, &part)?;
            Ok(part)
        })
    }
    pub fn resolve_activation(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        node: &Name,
        visit: Counter,
        expected_run: Counter,
    ) -> Result<Activation> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Executor,
                false,
            )?;
            resolve_activation_transition(
                tx,
                &run,
                revision,
                node,
                visit,
                expected_run,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )
        })
    }
}

fn finish_arm_outbox(
    tx: &mut dyn rx_ports::Transaction,
    message: Option<&Id>,
    host: &Name,
    attempt: &Id,
) -> Result<()> {
    if let Some(id) = message {
        let record = tx
            .outbox(id)?
            .ok_or(StoreError::Rejected(Reject::NotFound))?;
        let payload: Delivery = canonical::decode_json(
            &canonical::bytes(&record.document.value).map_err(domain_error)?,
        )
        .map_err(domain_error)?;
        if !matches!(payload,Delivery::Arm{host:ref h,attempt:ref a,..} if h==host && a==attempt) {
            return reject(Reject::InvalidInput);
        }
        if record.state != rx_ports::OutboxState::Delivered {
            tx.transition_outbox(
                id,
                rx_ports::OutboxState::EmitEntered,
                rx_ports::OutboxState::Delivered,
            )?;
        }
    }
    Ok(())
}

pub(super) fn begin_part_transition(
    tx: &mut dyn Transaction,
    cell: &Cell,
    run: &mut Run,
    revision: Counter,
    expected_budget: Counter,
    context: ProcessingContext<'_>,
) -> Result<PartSnapshot> {
    let ProcessingContext {
        identity,
        meta,
        now,
    } = context;
    active_run(tx, cell, run, identity, meta, now)?;
    check_revision(
        run.budget
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?
            .revision(),
        expected_budget,
    )?;
    if run.purpose != Some(Purpose::Production) {
        return reject(Reject::InvalidInput);
    }
    let part = PartAttempt {
        id: id(),
        run: run.id.clone(),
        ordinal: Counter(run.part_ids.len() as u64 + 1),
        disposition: PartDisposition::InProgress,
    };
    run.budget
        .as_mut()
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?
        .consume(Consumption::PartAttempt(part.id.clone()))
        .map_err(|_| StoreError::Rejected(Reject::BudgetExhausted))?;
    run.part_ids.push(part.id.clone());
    let part_revision = save(tx, "part", &part.id, None, PART, &part)?;
    save(tx, "run", &run.id, Some(revision), RUN, run)?;
    event(tx, "rx.event.part-created.v1", &part)?;
    Ok(PartSnapshot {
        revision: part_revision,
        part,
    })
}

pub(super) fn resolve_activation_transition(
    tx: &mut dyn Transaction,
    run: &Run,
    revision: Counter,
    node: &Name,
    visit: Counter,
    expected_run: Counter,
    context: ProcessingContext<'_>,
) -> Result<Activation> {
    let ProcessingContext {
        identity,
        meta,
        now,
    } = context;
    let run_id = &run.id;
    let identity_key = (run_id, node, visit);
    if let Some(record) = tx.get(&key("activation", identity_key))? {
        return decode(&record, ACTIVATION);
    }
    check_revision(revision, expected_run)?;
    let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
    active_run(tx, &cell, run, identity, meta, now)?;
    let step = cell
        .configuration
        .steps
        .iter()
        .find(|s| &s.id == node)
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
    let part_id = match run.purpose {
        Some(Purpose::Production) => {
            let ordinal = usize::try_from(
                visit
                    .0
                    .checked_sub(1)
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            )
            .map_err(|_| StoreError::Rejected(Reject::InvalidInput))?;
            Some(
                run.part_ids
                    .get(ordinal)
                    .cloned()
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            )
        }
        Some(Purpose::Setup) if visit == Counter(1) => None,
        _ => return reject(Reject::InvalidInput),
    };
    validate_part(tx, run, &part_id)?;
    if cell.configuration.process.is_some() {
        super::process::eligible_node(tx, run, &cell, node, visit)?;
    } else {
        predecessors_done(tx, run, &part_id, step)?;
    }
    let activation = Activation {
        id: id(),
        run: run.id.clone(),
        part: part_id,
        node: node.clone(),
        visit,
        slots: BTreeMap::new(),
    };
    save(
        tx,
        "activation",
        (&run.id, &activation.node, visit),
        None,
        ACTIVATION,
        &activation,
    )?;
    save(
        tx,
        "activationid",
        &activation.id,
        None,
        ACTIVATION,
        &activation,
    )?;
    save(tx, "run", &run.id, Some(revision), RUN, run)?;
    event(tx, "rx.event.activation-created.v1", &activation)?;
    Ok(activation)
}
