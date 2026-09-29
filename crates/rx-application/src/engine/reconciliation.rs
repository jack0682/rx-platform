use super::executor_peer::executor_scope;
use super::*;
use rx_domain::operation::Disposition;
use rx_ports::OutboxState;
const PLAN: &str = "rx.internal.reconciliation-request.v1";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Persist a coalesced read/query plan. It never issues a new production invocation.
    pub fn request_reconciliation(&mut self, identity: &Identity, operation: &Id) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            let (principal, _, _) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &clock.now(),
                },
                &work.cell,
            )?;
            if work.intent.kind == rx_domain::intent::Kind::ControlSession {
                return reject(Reject::CapabilityMissing);
            }
            if work.operation.disposition() == Disposition::Released {
                return Ok(work);
            }
            let k = key("reconciliation", operation);
            let old = tx.get(&k)?;
            let generation = if let Some(row) = &old {
                let prior: ReconciliationRequest = decode(row, PLAN)?;
                if prior.state == ReconciliationState::Pending {
                    return Ok(work);
                }
                prior.generation.increment().map_err(domain_error)?
            } else {
                Counter(1)
            };
            let request = ReconciliationRequest {
                id: id(),
                operation: operation.clone(),
                cell: work.cell.clone(),
                host: work.host.clone(),
                generation,
                requested_by: principal.id,
                state: ReconciliationState::Pending,
                issue: None,
            };
            tx.put(&k, old.map(|v| v.revision), &doc(PLAN, &request)?)?;
            event(tx, "rx.event.reconciliation-requested.v1", &request)?;
            Ok(work)
        })
    }
    pub fn pending_reconciliations(
        &mut self,
        identity: &Identity,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<ReconciliationRequest>> {
        if limit == 0 || limit > 128 {
            return reject(Reject::InvalidInput);
        }
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let mut plans = vec![];
            for row in tx.scan("reconciliation/")? {
                let item: ReconciliationRequest = decode(&row, PLAN)?;
                if item.host == p.id
                    && p.cells.contains(&item.cell)
                    && item.state == ReconciliationState::Pending
                    && after.is_none_or(|a| item.operation > *a)
                {
                    plans.push(item);
                }
            }
            plans.sort_by(|a, b| a.operation.cmp(&b.operation));
            plans.truncate(limit);
            Ok(plans)
        })
    }
    pub fn plan_reconciliation(
        &mut self,
        identity: &Identity,
        operation: &Id,
        request: &Id,
    ) -> Result<ReconciliationPlan> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, plan): (_, ReconciliationRequest) =
                load(tx, "reconciliation", operation, PLAN)?;
            authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            if plan.id != *request
                || plan.state != ReconciliationState::Pending
                || plan.host != identity.principal
            {
                return reject(Reject::StaleRevision);
            }
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            let (cell_revision, _): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&work.cell, &work.host), HOST)?;
            if host.session != identity.session || plan.cell != work.cell || plan.host != work.host
            {
                return reject(Reject::ContinuityUnproven);
            }
            let authorize = super::delivery::key_id(operation, "authorize");
            let receipt_message = if tx.outbox(&authorize)?.is_some_and(|r| {
                matches!(r.state, OutboxState::EmitEntered | OutboxState::Delivered)
            }) {
                Some(authorize)
            } else if tx.outbox(operation)?.is_some_and(|r| {
                matches!(r.state, OutboxState::EmitEntered | OutboxState::Delivered)
            }) {
                Some(operation.clone())
            } else {
                None
            };
            let settlement = super::settlement::pending_authorization(tx, operation)?;
            Ok(ReconciliationPlan {
                settlement,
                request: plan,
                work,
                cell_revision,
                receipt_message,
                host,
            })
        })
    }
    pub fn update_reconciliation(
        &mut self,
        identity: &Identity,
        operation: &Id,
        request: &Id,
        state: ReconciliationState,
        issue: Option<ReconciliationIssue>,
    ) -> Result<()> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (revision, mut plan): (_, ReconciliationRequest) =
                load(tx, "reconciliation", operation, PLAN)?;
            authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            if plan.id != *request || plan.host != identity.principal {
                return reject(Reject::StaleRevision);
            }
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&plan.cell, &plan.host), HOST)?;
            if host.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            if state == ReconciliationState::Complete {
                let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
                if work.operation.disposition() != Disposition::Released {
                    return reject(Reject::ConditionUnknown);
                }
            }
            if plan.state == state && plan.issue == issue {
                return Ok(());
            }
            if plan.state != ReconciliationState::Pending {
                return reject(Reject::StaleRevision);
            }
            plan.state = state;
            plan.issue = issue;
            save(tx, "reconciliation", operation, Some(revision), PLAN, &plan)?;
            event(tx, "rx.event.reconciliation-updated.v1", &plan)
        })
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Refresh retained native evidence against current postconditions without
    /// replaying native work or restoring a lost authorization context.
    pub fn refresh_reconciliation_completion(
        &mut self,
        identity: &Identity,
        operation: &Id,
        request: &Id,
    ) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, plan): (_, ReconciliationRequest) =
                load(tx, "reconciliation", operation, PLAN)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&work.cell, &work.host), HOST)?;
            if plan.id != *request
                || plan.state != ReconciliationState::Pending
                || plan.host != principal.id
                || work.host != principal.id
                || plan.cell != work.cell
                || host.session != identity.session
                || host.delivery_journal != work.host_journal
            {
                return reject(Reject::ContinuityUnproven);
            }
            if work.operation.outcome() == Outcome::None
                && work.operation.integrity() == Integrity::Valid
            {
                let (_, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
                let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
                let postconditions = match &work.completion {
                    CompletionRule::Native { postconditions, .. }
                    | CompletionRule::NativeOutcomes { postconditions, .. } => postconditions,
                    _ => return Ok(work),
                };
                if !postconditions.is_empty()
                    && cell.blocks.is_empty()
                    && cell.epoch == permit.epoch
                    && cell.scope_epochs == permit.scopes
                    && host.boot_id == permit.host_boot
                {
                    match evaluate(tx, &cell, postconditions, &now) {
                        Ok(_) => super::evidence::reapply_correlated_native(
                            tx, &principal, operation, &now,
                        )?,
                        Err(StoreError::Rejected(
                            Reject::ConditionFailed | Reject::ConditionUnknown,
                        )) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            Ok(load(tx, "work", operation, WORK)?.1)
        })
    }

    /// Retain authenticated observation facts even when they cannot justify handover.
    pub fn record_reconciliation_observations(
        &mut self,
        identity: &Identity,
        operation: &Id,
        request: &Id,
        observations: &[HandoverObservation],
    ) -> Result<()> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, plan): (_, ReconciliationRequest) =
                load(tx, "reconciliation", operation, PLAN)?;
            authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            if plan.id != *request
                || plan.host != identity.principal
                || plan.state != ReconciliationState::Pending
            {
                return reject(Reject::StaleRevision);
            }
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&plan.cell, &plan.host), HOST)?;
            if host.session != identity.session
                || observations.len() != 3
                || observations
                    .iter()
                    .map(|o| &o.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != 3
                || observations
                    .iter()
                    .any(|o| o.operation != *operation || o.host_boot != host.boot_id)
            {
                return reject(Reject::InvalidInput);
            }
            for observation in observations {
                let k = key("handover", &observation.id);
                let value = doc("rx.internal.handover-observation.v1", observation)?;
                if let Some(old) = tx.get(&k)?
                    && old.document != value
                {
                    event(
                        tx,
                        "rx.event.handover-observation-conflict.v1",
                        &(request, observations),
                    )?;
                    invalidate_closure(tx, &plan.cell, BlockReason::IntegrityConflict)?;
                    return Ok(Err(StoreError::Integrity(
                        "handover observation identity conflict".into(),
                    )));
                }
            }
            let mut changed = false;
            for observation in observations {
                let k = key("handover", &observation.id);
                if tx.get(&k)?.is_none() {
                    tx.put(
                        &k,
                        None,
                        &doc("rx.internal.handover-observation.v1", observation)?,
                    )?;
                    changed = true;
                }
            }
            if changed {
                event(
                    tx,
                    "rx.event.reconciliation-observations-recorded.v1",
                    &(request, observations),
                )?;
            }
            Ok(Ok(()))
        })?
    }
}
