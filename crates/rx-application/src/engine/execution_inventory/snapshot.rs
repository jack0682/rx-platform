use super::*;
use v2::snapshot as wire;
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// One P control cut. The immutable Plan2 reference is never replaced with a v1 recipe.
    pub fn execution_snapshot_v2(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        visit: Counter,
    ) -> Result<wire::Snapshot> {
        let meta = &self.installation;
        let clock = &self.clock;
        let (mut snapshot, sequence) = self.repository.transact_at_control_cut(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let (_, cell_revision, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            execution_session::require(tx, identity, meta, &now, &cell)?;
            let configuration = run_configuration::read(tx, &run, &cell.configuration)?;
            let plan = execution_configuration::plan(&configuration)?
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            let part_id = visit
                .0
                .checked_sub(1)
                .and_then(|n| run.part_ids.get(n as usize))
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let (_, part): (_, v2::executor::PartBinding) = load(
                tx,
                "executionpart",
                part_id,
                v2::executor::PART_BINDING_SCHEMA,
            )?;
            let mut historical = cell.clone();
            historical.configuration = configuration.clone();
            let (process_checkpoint, progress) =
                process::process_view(tx, &run, &historical, visit)?;
            let admission = active_run(tx, &cell, &run, identity, meta, &now).and_then(|()| {
                let (_, binding): (_, data::RunBinding) =
                    load(tx, "executionrun", run_id, BINDING)?;
                object_reference_current(tx, &cell, &binding, visit).map(|_| ())
            });
            let admission_reason = match admission {
                Ok(()) => None,
                Err(StoreError::Rejected(r)) => Some(r),
                Err(e) => return Err(e),
            };
            let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)?;
            let until = now
                .ticks_ns
                .0
                .checked_add(100_000_000)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                .min(session.expires_at.ticks_ns.0);
            let run = queries::read_run_checkpoint(tx, identity, meta, &now, run_id)?;
            Ok(wire::Snapshot {
                schema: name(wire::SNAPSHOT_SCHEMA),
                configuration: configuration.reference().map_err(StoreError::Invalid)?,
                plan,
                part,
                context: ExecutionSnapshot {
                    schema: name(wire::CONTEXT_SCHEMA),
                    installation: meta.id.clone(),
                    store_generation: meta.store_generation.clone(),
                    runtime_boot: meta.runtime_boot.clone(),
                    sequence: Counter(0),
                    caller_session: identity.session.clone(),
                    definition: configuration.definition,
                    envelope: configuration.envelope,
                    cell_revision,
                    cell_epoch: cell.epoch,
                    scope_epochs: cell.scope_epochs,
                    checked_at: now.clone(),
                    valid_until: TimePoint {
                        clock_id: now.clock_id,
                        ticks_ns: Counter(until),
                    },
                    run,
                    visit,
                    resolved: configuration.recipe,
                    process_checkpoint,
                    progress,
                    request_admission_allowed: admission_reason.is_none(),
                    admission_reason,
                },
            })
        })?;
        snapshot.context.sequence = sequence;
        snapshot.validate().map_err(StoreError::Integrity)?;
        Ok(snapshot)
    }
}
