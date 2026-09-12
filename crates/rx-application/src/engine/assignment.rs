use super::*;
use rx_process_contract::assignment::{
    self, AttemptStatus, Candidate, Cardinality, PendingAttempt, View,
};
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn executor_assignment(&mut self, identity: &Identity, cell_id: &Name) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        let (mut view, sequence) = self.repository.transact_at_control_cut(|tx| {
            let now = clock.now();
            let (principal, revision, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                cell_id,
            )?;
            let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)?;
            let rows = tx.scan("run/")?;
            // Storage currently materializes a scan. Refuse an incomplete/oversized discovery;
            // this bound is not claimed as a bound on the storage allocation itself.
            if rows.len() > 10_000 {
                return Err(StoreError::Unavailable("assignment scan limit".into()));
            }
            let mut candidates = Vec::new();
            for row in rows {
                let run: Run = decode(&row, RUN)?;
                if row.key != key("run", &run.id) {
                    return Err(StoreError::Integrity("assignment Run key differs".into()));
                }
                if run.cell != *cell_id
                    || matches!(run.state, RunState::Completed | RunState::Abandoned)
                    || (run.state == RunState::Prepared && run.pending_attempt.is_none())
                {
                    continue;
                }
                let pending_attempt = if let Some(id) = &run.pending_attempt {
                    let (_, attempt): (_, StartAttempt) = load(tx, "attempt", id, ATTEMPT)?;
                    if attempt.id != *id || attempt.run != run.id || attempt.cell != run.cell {
                        return Err(StoreError::Integrity(
                            "assignment StartAttempt relation differs".into(),
                        ));
                    }
                    Some(PendingAttempt {
                        id: attempt.id,
                        status: match attempt.status {
                            StartStatus::Pending => AttemptStatus::Pending,
                            StartStatus::Arming => AttemptStatus::Arming,
                            StartStatus::Started => AttemptStatus::Started,
                            StartStatus::Rejected => AttemptStatus::Rejected,
                        },
                        executor_session: attempt.executor_session,
                        valid_until: attempt.valid_until,
                    })
                } else {
                    None
                };
                let cfg = run_configuration::read(tx, &run, &cell.configuration)?;
                let configuration_current = process_change::config_ref(&cfg)?
                    == process_change::config_ref(&cell.configuration)?;
                candidates.push(Candidate {
                    run: run.id,
                    revision: row.revision,
                    state: run.state,
                    purpose: run.purpose,
                    definition: cfg.definition,
                    resolved: cfg.recipe,
                    executor_session: run.executor_session,
                    mandate: run.mandate,
                    pending_attempt,
                    configuration_current,
                });
                if candidates.len() == 2 {
                    break;
                }
            }
            let until = now
                .ticks_ns
                .0
                .checked_add(100_000_000)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                .min(session.expires_at.ticks_ns.0);
            Ok(View {
                schema: name(assignment::SCHEMA),
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                sequence: Counter(0),
                caller_session: identity.session.clone(),
                cell: cell_id.clone(),
                executor: principal.id,
                definition: cell.configuration.definition,
                cell_revision: revision,
                cell_epoch: cell.epoch,
                scope_epochs: cell.scope_epochs,
                checked_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id,
                    ticks_ns: Counter(until),
                },
                cardinality: match candidates.len() {
                    0 => Cardinality::None,
                    1 => Cardinality::Single,
                    _ => Cardinality::Ambiguous,
                },
                candidates,
            })
        })?;
        view.sequence = sequence;
        assignment::validate(&view).map_err(StoreError::Integrity)?;
        Ok(view)
    }
}
