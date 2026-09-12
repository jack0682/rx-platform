use super::*;
use rx_domain::operation::{Disposition, Phase, ReleaseConditions};
const HANDOVER: &str = "rx.internal.handover-observation.v1";

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn release_resources(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: ReleaseResources,
    ) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, mut work): (_, Work) = load(tx, "work", &command.operation, WORK)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&work.cell),
                Role::Host,
                false,
            )?;
            if principal.id != work.host {
                return reject(Reject::Forbidden);
            }
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Resource.Release",
                request_key,
                &(&command.operation, &command.observations),
            )?;
            if let Some(old) = prior(tx, &scope, fingerprint, WORK)? {
                return Ok(old);
            }
            if work.operation.disposition() == Disposition::Released {
                remember(tx, &scope, fingerprint, WORK, &work)?;
                return Ok(work);
            }
            check_revision(work.operation.revision(), command.expected_operation)?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
            check_revision(cell_revision, command.expected_cell)?;
            ready(tx, &cell, &now)?;
            let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&work.cell, &work.host), HOST)?;
            if host.session != identity.session
                || cell.epoch != permit.epoch
                || cell.scope_epochs != permit.scopes
                || host.boot_id != permit.host_boot
                || work.operation.phase() != Phase::Settled
                || work.operation.integrity() != Integrity::Valid
            {
                return reject(Reject::ContinuityUnproven);
            }
            let mut native = None;
            for id in work.operation.evidence_ids() {
                if let Some(row) = tx.get(&key("evidence", id))?
                    && row.document.schema.as_str() == "rx.internal.native-evidence.v1"
                {
                    let evidence: NativeEvidence = decode(&row, "rx.internal.native-evidence.v1")?;
                    if evidence.operation == command.operation
                        && work.invocation.as_ref() == Some(&evidence.invocation)
                    {
                        native = Some(evidence);
                    }
                }
            }
            let native = native.ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            let expected: BTreeSet<_> = ["no-pending", "control", "support"]
                .into_iter()
                .map(|s| name(format!("handover/{}/{s}", command.operation)))
                .collect();
            if command.observations.len() != 3
                || command
                    .observations
                    .iter()
                    .map(|o| o.source.clone())
                    .collect::<BTreeSet<_>>()
                    != expected
                || command
                    .observations
                    .iter()
                    .map(|o| o.id.clone())
                    .collect::<BTreeSet<_>>()
                    .len()
                    != 3
            {
                return reject(Reject::InvalidInput);
            }
            let mut evidence_ids = Vec::new();
            for observation in &command.observations {
                let age = now
                    .age_ns(&observation.observed_at)
                    .and_then(|n| n.checked_add(observation.uncertainty_ns.0));
                if observation.operation != command.operation
                    || Some(&observation.invocation) != work.invocation.as_ref()
                    || observation.profile_digest != work.intent.profile_digest
                    || observation.device_session != native.device_session
                    || observation.host_boot != host.boot_id
                    || observation.schema.as_str() != "rx.handover.v1"
                    || !observation.value
                    || !observation.quality_good
                    || !observation.origin_age_bounded
                    || age.is_none_or(|n| n > work.handover_max_age_ns.0)
                    || observation.observed_at.clock_id != native.captured_at.clock_id
                    || observation.observed_at.ticks_ns < native.captured_at.ticks_ns
                {
                    return reject(Reject::ConditionUnknown);
                }
                let key_ = key("handover", &observation.id);
                let incoming = doc(HANDOVER, observation)?;
                if let Some(old) = tx.get(&key_)? {
                    if old.document != incoming {
                        return Err(StoreError::KeyConflict);
                    }
                } else {
                    tx.put(&key_, None, &incoming)?;
                }
                evidence_ids.push(observation.id.clone());
            }
            for resource_id in &work.intent.resource_set {
                let (rr, mut resource): (_, Resource) =
                    load(tx, "resource", resource_id, RESOURCE)?;
                if resource.holder.as_ref() != Some(work.operation.id()) {
                    return reject(Reject::Busy);
                }
                resource.holder = None;
                resource.quarantined = false;
                save(tx, "resource", resource_id, Some(rr), RESOURCE, &resource)?;
                event(tx, "rx.event.resource-released.v1", &resource)?;
            }
            work.operation
                .release(
                    ReleaseConditions {
                        no_residual_native: true,
                        control_handover_confirmed: true,
                        support_handover_confirmed: true,
                    },
                    evidence_ids,
                )
                .map_err(domain_error)?;
            save(tx, "work", work.operation.id(), Some(revision), WORK, &work)?;
            remember(tx, &scope, fingerprint, WORK, &work)?;
            event(tx, "rx.event.operation-resources-released.v1", &work)?;
            Ok(work)
        })
    }
    pub fn complete_part(
        &mut self,
        identity: &Identity,
        request_key: &str,
        part_id: &Id,
        expected_run: Counter,
    ) -> Result<PartAttempt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (part_revision, mut part): (_, PartAttempt) = load(tx, "part", part_id, PART)?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &part.run, RUN)?;
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
                "Cell.CompletePartAttempt",
                request_key,
                part_id,
            )?;
            if let Some(old) = prior(tx, &scope, fingerprint, PART)? {
                return Ok(old);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            if part.disposition != PartDisposition::ConfirmedCompleted {
                check_revision(run_revision, expected_run)?;
                active_run(tx, &cell, &run, identity, meta, &now)?;
            }
            let result = complete_part_transition(
                tx,
                &cell,
                &mut run,
                run_revision,
                &mut part,
                part_revision,
            )?;
            remember(tx, &scope, fingerprint, PART, &part)?;
            Ok(result.part)
        })
    }
}

pub(super) fn complete_part_transition(
    tx: &mut dyn Transaction,
    cell: &Cell,
    run: &mut Run,
    run_revision: Counter,
    part: &mut PartAttempt,
    part_revision: Counter,
) -> Result<PartSnapshot> {
    if part.disposition == PartDisposition::ConfirmedCompleted {
        return Ok(PartSnapshot {
            revision: part_revision,
            part: part.clone(),
        });
    }
    if part.disposition != PartDisposition::InProgress {
        return reject(Reject::ConditionUnknown);
    }
    if let Some(process) = &cell.configuration.process {
        let (_, view) = super::process::process_view(tx, run, cell, part.ordinal)?;
        let next =
            rx_process_contract::frontier::plan(process, &view).map_err(StoreError::Integrity)?;
        if next.state != rx_process_contract::frontier::State::Completed {
            return reject(Reject::ConditionUnknown);
        }
    } else {
        for step in &cell.configuration.steps {
            let (_, activation): (_, Activation) = load(
                tx,
                "activation",
                (&run.id, &step.id, part.ordinal),
                ACTIVATION,
            )?;
            let op = activation
                .slots
                .get(&name("main"))
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            let (_, work): (_, Work) = load(tx, "work", op, WORK)?;
            if work.operation.outcome() != Outcome::Succeeded
                || work.operation.integrity() != Integrity::Valid
                || work.operation.disposition() != Disposition::Released
            {
                return reject(Reject::ConditionUnknown);
            }
        }
    }
    part.disposition = PartDisposition::ConfirmedCompleted;
    let revision = save(tx, "part", &part.id, Some(part_revision), PART, &part)?;
    let mut all_done = true;
    for id in &run.part_ids {
        let (_, item): (_, PartAttempt) = load(tx, "part", id, PART)?;
        all_done &= item.disposition == PartDisposition::ConfirmedCompleted;
    }
    if all_done && run.budget.as_ref().is_some_and(|b| b.remaining().0 == 0) {
        run.state = RunState::Completed;
        if let Some(id) = &run.mandate {
            let (revision, mut mandate): (_, Mandate) = load(tx, "mandate", id, MANDATE)?;
            mandate.state = MandateState::Exhausted;
            save(tx, "mandate", id, Some(revision), MANDATE, &mandate)?;
        }
    }
    save(tx, "run", &run.id, Some(run_revision), RUN, &run)?;
    event(tx, "rx.event.part-completed.v1", part)?;
    Ok(PartSnapshot {
        revision,
        part: part.clone(),
    })
}
