use super::*;
use crate::operator_start::{
    AttemptContext, AttemptRequest, ContextRequest, DeadlineStatus, StartContext,
};

pub(super) struct StartBasis<'a> {
    pub run_revision: Counter,
    pub run: &'a Run,
    pub cell_revision: Counter,
    pub cell: &'a Cell,
}
pub(super) struct ValidatedStart {
    pub executor_session: Id,
    pub host_boots: BTreeMap<Name, Id>,
    pub next_run_revision: Counter,
    pub valid_until: TimePoint,
    pub clear_blocks: Vec<Id>,
}

/// Shared read-only checks. No budget, attempt, request receipt, outbox or authority is written here.
pub(super) fn validate_candidate(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    basis: StartBasis<'_>,
    command: &StartRun,
) -> Result<ValidatedStart> {
    let run = basis.run;
    let cell = basis.cell;
    authorize(
        tx,
        identity,
        meta,
        now,
        Some(&run.cell),
        Role::Operator,
        true,
    )?;
    check_revision(basis.cell_revision, command.expected_cell)?;
    check_revision(basis.run_revision, command.expected_run)?;
    run_configuration::require_current(tx, run, cell)?;
    if run.state != RunState::Prepared {
        return reject(Reject::MandateRevoked);
    }
    if run.pending_attempt.is_some() {
        return reject(Reject::Busy);
    }
    // A scalar cell operating context cannot describe competing active purposes.
    for row in tx.scan("run/")? {
        let active: Run = decode(&row, RUN)?;
        if active.cell == run.cell
            && active.id != run.id
            && active.state == RunState::Executing
            && active.purpose != Some(command.purpose)
        {
            return reject(Reject::Busy);
        }
    }
    let unit = if command.purpose == Purpose::Production {
        BudgetUnit::PartAttempt
    } else {
        BudgetUnit::OperationCount
    };
    if command.envelope_digest != cell.configuration.envelope.sha256
        || command.budget_unit != unit
        || command.budget_limit.0 == 0
        || command.budget_limit > cell.configuration.maximum_budget
    {
        return reject(Reject::InvalidInput);
    }
    if let Some(budget) = &run.budget
        && (budget.unit() != unit
            || budget.limit() != command.budget_limit
            || run.purpose != Some(command.purpose))
    {
        return reject(Reject::InvalidInput);
    }
    ready(tx, cell, now)?;
    qualification_activation::purpose(tx, cell, command.purpose)?;
    evaluate(tx, cell, &cell.configuration.start_conditions, now)?;
    let executor = current_session(
        tx,
        &cell.configuration.executor,
        meta,
        now,
        Role::Executor,
        &run.cell,
    )?;
    let mut host_boots = BTreeMap::new();
    for host in &cell.configuration.hosts {
        let registration = prepared_host(tx, cell, host, meta, now)?;
        host_boots.insert(host.clone(), registration.boot_id);
    }
    Ok(ValidatedStart {
        executor_session: executor.id,
        host_boots,
        next_run_revision: basis.run_revision.increment().map_err(domain_error)?,
        valid_until: TimePoint {
            clock_id: now.clock_id.clone(),
            ticks_ns: Counter(
                now.ticks_ns
                    .0
                    .checked_add(cell.configuration.start_timeout_ns.0)
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            ),
        },
        clear_blocks: qualification_activation::arm_clear(tx, cell)?,
    })
}

fn current_installation(tx: &mut dyn Transaction, meta: &Installation) -> Result<()> {
    let row = tx
        .get(&name("installation/current"))?
        .ok_or_else(|| StoreError::Integrity("installation missing".into()))?;
    let current: Installation = decode(&row, "rx.internal.installation.v1")?;
    if canonical::bytes(&current).map_err(domain_error)?
        != canonical::bytes(meta).map_err(domain_error)?
    {
        return Err(StoreError::Integrity(
            "operator start read belongs to another installation".into(),
        ));
    }
    Ok(())
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn operator_start_context(
        &mut self,
        identity: &Identity,
        input: ContextRequest,
    ) -> Result<StartContext> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize_read(tx, identity, meta, &now, &input.cell)?;
            if !actor.roles.contains(&Role::Operator) {
                return reject(Reject::Forbidden);
            }
            current_installation(tx, meta)?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            let (run_revision, run): (_, Run) = load(tx, "run", &input.run, RUN)?;
            if cell.configuration.id != input.cell || run.id != input.run || run.cell != input.cell
            {
                return reject(Reject::Forbidden);
            }
            let request = StartRun {
                run: run.id.clone(),
                envelope_digest: cell.configuration.envelope.sha256,
                purpose: input.purpose,
                budget_unit: if input.purpose == Purpose::Production {
                    BudgetUnit::PartAttempt
                } else {
                    BudgetUnit::OperationCount
                },
                budget_limit: input.budget_limit,
                expected_cell: cell_revision,
                expected_run: run_revision,
            };
            let blocking_reason = match validate_candidate(
                tx,
                identity,
                meta,
                &now,
                StartBasis {
                    run_revision,
                    run: &run,
                    cell_revision,
                    cell: &cell,
                },
                &request,
            ) {
                Ok(_) => None,
                Err(StoreError::Rejected(reason)) => Some(reason),
                Err(error) => return Err(error),
            };
            Ok(StartContext {
                installation: meta.clone(),
                checked_at: now,
                cell: input.cell,
                cell_revision,
                epoch: cell.epoch,
                scope_epochs: cell.scope_epochs.clone(),
                configuration_digest: crate::runtime_invalidation::configuration_digest(
                    &cell.configuration,
                )
                .map_err(StoreError::Integrity)?,
                environment: cell.configuration.environment,
                commissioning: cell.commissioning,
                envelope: cell.configuration.envelope,
                recipe: cell.configuration.recipe,
                site_config_digest: cell.configuration.site_config_digest,
                maximum_budget: cell.configuration.maximum_budget,
                run_revision,
                run,
                request,
                can_request: blocking_reason.is_none(),
                blocking_reason,
            })
        })
    }

    pub fn operator_start_attempt(
        &mut self,
        identity: &Identity,
        input: AttemptRequest,
    ) -> Result<AttemptContext> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            authorize_read(tx, identity, meta, &now, &input.cell)?;
            current_installation(tx, meta)?;
            let (run_revision, run): (_, Run) = load(tx, "run", &input.run, RUN)?;
            let (_, attempt): (_, StartAttempt) = load(tx, "attempt", &input.id, ATTEMPT)?;
            if run.id != input.run
                || run.cell != input.cell
                || attempt.id != input.id
                || attempt.cell != input.cell
                || attempt.run != input.run
            {
                return reject(Reject::Forbidden);
            }
            let deadline_status = if now.clock_id != attempt.valid_until.clock_id {
                DeadlineStatus::ClockChanged
            } else if now.ticks_ns >= attempt.valid_until.ticks_ns {
                DeadlineStatus::Elapsed
            } else {
                DeadlineStatus::WithinDeadline
            };
            Ok(AttemptContext {
                installation: meta.clone(),
                checked_at: now,
                run_revision,
                run,
                attempt,
                deadline_status,
            })
        })
    }
}
