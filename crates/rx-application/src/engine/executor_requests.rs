use super::*;
use super::{
    executor_peer::executor_scope,
    workflow::{begin_part_transition, resolve_activation_transition},
};
use crate::checkpoint_artifact::{ActivationSnapshot, activation_snapshot};
const PART_REPLY: &str = "rx.internal.part-attempt-reply.v1";
const ACTIVATION_REPLY: &str = "rx.internal.activation-reply.v1";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn executor_begin_part(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: BeginPartRequest,
    ) -> Result<PartSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (principal, cell_revision, cell) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &command.cell,
            )?;
            let (revision, mut run): (_, Run) = load(tx, "run", &command.run, RUN)?;
            if run.cell != command.cell {
                return reject(Reject::InvalidInput);
            }
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Cell.BeginPartAttempt",
                request_key,
                &command,
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, PART_REPLY)? {
                return Ok(saved);
            }
            if run.mandate.as_ref() != Some(&command.mandate) {
                return reject(Reject::MandateRevoked);
            }
            if let Some(expected) = command.expected_cell {
                check_revision(cell_revision, expected)?;
            }
            let result = begin_part_transition(
                tx,
                &cell,
                &mut run,
                revision,
                command.expected_budget,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )?;
            remember(tx, &scope, fingerprint, PART_REPLY, &result)?;
            Ok(result)
        })
    }
    pub fn executor_resolve_activation(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: ResolveActivationRequest,
    ) -> Result<ActivationSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", &command.run, RUN)?;
            let (principal, _, _) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Workflow.ResolveActivation",
                request_key,
                &command,
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, ACTIVATION_REPLY)? {
                return Ok(saved);
            }
            let activation = resolve_activation_transition(
                tx,
                &run,
                revision,
                &command.node,
                command.visit,
                command.expected_run,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )?;
            let result = activation_snapshot(tx, &activation, &run.cell)?;
            remember(tx, &scope, fingerprint, ACTIVATION_REPLY, &result)?;
            Ok(result)
        })
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn executor_submit(
        &mut self,
        identity: &Identity,
        request_key: &str,
        mut command: ExecutorSubmitRequest,
    ) -> Result<AdmissionReceipt> {
        command.work.intent = command.work.intent.normalized().map_err(domain_error)?;
        let clock = &self.clock;
        let meta = &self.installation;
        let work = self.repository.transact(|tx| {
            let now = clock.now();
            let (principal, _, _) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &command.cell,
            )?;
            let (activation_revision, mut activation): (_, Activation) =
                load(tx, "activationid", &command.work.activation, ACTIVATION)?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &activation.run, RUN)?;
            if run.cell != command.cell
                || command.work.run != run.id
                || command.work.part != activation.part
            {
                return reject(Reject::InvalidInput);
            }
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Cell.SubmitOperation",
                request_key,
                &command,
            )?;
            if let Some(saved) = prior::<Work>(tx, &scope, fingerprint, WORK)? {
                return Ok(saved);
            }
            // Continuous control needs its separate session/deadman lifecycle before public admission.
            if command.work.intent.kind == rx_domain::intent::Kind::ControlSession {
                return reject(Reject::CapabilityMissing);
            }
            if let Some(operation) = activation.slots.get(&command.work.slot) {
                let (_, existing): (_, Work) = load(tx, "work", operation, WORK)?;
                let (_, permit): (_, Permit) = load(tx, "permit", &existing.permit, PERMIT)?;
                if existing.operation.id() != operation
                    || existing.cell != command.cell
                    || existing.run != run.id
                    || existing.activation != activation.id
                    || existing.part != activation.part
                    || existing.slot != command.work.slot
                    || permit.operation != *operation
                    || permit.cell != command.cell
                    || permit.intent_digest != existing.operation.intent_digest()
                    || existing.intent.digest().map_err(domain_error)?
                        != existing.operation.intent_digest()
                {
                    return Err(StoreError::Integrity(
                        "existing slot/work/permit binding differs".into(),
                    ));
                }
                if existing.intent.digest().map_err(domain_error)?
                    != command.work.intent.digest().map_err(domain_error)?
                {
                    return Err(StoreError::KeyConflict);
                }
                if permit.mandate != command.mandate {
                    return reject(Reject::MandateRevoked);
                }
                remember(tx, &scope, fingerprint, WORK, &existing)?;
                return Ok(existing);
            }
            if run.mandate.as_ref() != Some(&command.mandate) {
                return reject(Reject::MandateRevoked);
            }
            let work = super::dispatch::submit_transition(
                tx,
                super::dispatch::DispatchState {
                    activation: &mut activation,
                    activation_revision,
                    run: &mut run,
                    run_revision,
                },
                &command.work,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
            )?;
            remember(tx, &scope, fingerprint, WORK, &work)?;
            Ok(work)
        })?;
        // Tracking commits this immutable receipt with T1. A failed read cannot undo that commit;
        // the caller retains the same key and recovers the original receipt on retry.
        self.admission_receipt(identity, work.operation.id())
    }
    pub fn executor_work(&mut self, identity: &Identity, operation: &Id) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &work.cell,
            )?;
            Ok(work)
        })
    }
}
