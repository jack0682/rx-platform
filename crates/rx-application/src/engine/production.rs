use super::executor_peer::executor_scope;
use super::*;
use rx_process_contract::production::{self, CompletePart, Part, View};
const REPLY: &str = "rx.internal.complete-part-reply.v1";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn production_view(&mut self, identity: &Identity, run_id: &Id) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        let (mut view, sequence) = self.repository.transact_at_control_cut(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let (_, cell_revision, cell) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            let configuration = run_configuration::read(tx, &run, &cell.configuration)?;
            let admission_allowed = match active_run(tx, &cell, &run, identity, meta, &now) {
                Ok(()) => true,
                Err(StoreError::Rejected(_)) => false,
                Err(e) => return Err(e),
            };
            let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)?;
            let until = now
                .ticks_ns
                .0
                .checked_add(100_000_000)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                .min(session.expires_at.ticks_ns.0);
            let mut parts = vec![];
            for id in &run.part_ids {
                let (revision, part): (_, PartAttempt) = load(tx, "part", id, PART)?;
                parts.push(Part {
                    id: part.id,
                    run: part.run,
                    ordinal: part.ordinal,
                    revision,
                    disposition: part.disposition,
                });
            }
            let run = super::queries::read_run_checkpoint(tx, identity, meta, &now, run_id)?;
            Ok(View {
                schema: name(production::SCHEMA),
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                sequence: Counter(0),
                caller_session: identity.session.clone(),
                definition: configuration.definition,
                resolved: configuration.recipe,
                cell_revision,
                cell_epoch: cell.epoch,
                scope_epochs: cell.scope_epochs,
                checked_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id,
                    ticks_ns: Counter(until),
                },
                run,
                parts,
                admission_allowed,
            })
        })?;
        view.sequence = sequence;
        production::validate(&view).map_err(StoreError::Integrity)?;
        Ok(view)
    }
    pub fn executor_complete_part(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: CompletePart,
    ) -> Result<PartSnapshot> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (principal, _, cell) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &command.cell,
            )?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &command.run, RUN)?;
            let (part_revision, mut part): (_, PartAttempt) =
                load(tx, "part", &command.part, PART)?;
            if run.cell != command.cell || part.run != run.id || !run.part_ids.contains(&part.id) {
                return reject(Reject::InvalidInput);
            }
            let (scope, fingerprint) =
                request(meta, &principal, "Production.CompletePart", key_, &command)?;
            if let Some(saved) = prior(tx, &scope, fingerprint, REPLY)? {
                return Ok(saved);
            }
            if part.disposition != PartDisposition::ConfirmedCompleted {
                check_revision(run_revision, command.expected_run)?;
                check_revision(part_revision, command.expected_part)?;
                active_run(tx, &cell, &run, identity, meta, &now)?;
            }
            let result = super::handover::complete_part_transition(
                tx,
                &cell,
                &mut run,
                run_revision,
                &mut part,
                part_revision,
            )?;
            remember(tx, &scope, fingerprint, REPLY, &result)?;
            Ok(result)
        })
    }
}
