use super::executor_peer::executor_scope;
use super::*;
const VIEW_MAX_AGE_NS: u64 = 100_000_000;
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// A live read cut for one declared graph/visit. This is not a dispatch permit.
    pub fn execution_snapshot(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        visit: Counter,
    ) -> Result<ExecutionSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        let (mut snapshot, sequence) = self.repository.transact_at_control_cut(|tx| {
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
            let mut historical = cell.clone();
            historical.configuration = configuration.clone();
            let (process_checkpoint, progress) =
                super::process::process_view(tx, &run, &historical, visit)?;
            let admission_reason = match active_run(tx, &cell, &run, identity, meta, &now) {
                Ok(()) => None,
                Err(StoreError::Rejected(reason)) => Some(reason),
                Err(error) => return Err(error),
            };
            let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)?;
            let until = now
                .ticks_ns
                .0
                .checked_add(VIEW_MAX_AGE_NS)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                .min(session.expires_at.ticks_ns.0);
            let run = super::queries::read_run_checkpoint(tx, identity, meta, &now, run_id)?;
            Ok(ExecutionSnapshot {
                schema: name(rx_process_contract::execution::SNAPSHOT_SCHEMA),
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                sequence: Counter(0),
                caller_session: identity.session.clone(),
                definition: configuration.definition.clone(),
                envelope: configuration.envelope.clone(),
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
            })
        })?;
        snapshot.sequence = sequence;
        Ok(snapshot)
    }
    /// Only run-owned checkpoints and the current matching resolved process may be read here.
    pub fn executor_artifact(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        reference: &ArtifactRef,
    ) -> Result<Vec<u8>> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let (_, _, cell) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            if reference.schema_id.as_str() == crate::checkpoint_artifact::SCHEMA {
                return crate::checkpoint_artifact::read_artifact(tx, run_id, reference);
            }
            let configuration = run_configuration::read(tx, &run, &cell.configuration)?;
            if reference != &configuration.recipe || reference.sha256 != run.recipe_digest {
                return reject(Reject::UnsupportedSchema);
            }
            let process = configuration
                .process
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            let bytes = canonical::bytes(process).map_err(domain_error)?;
            if bytes.len() as u64 != reference.size_bytes.0
                || rx_process_contract::frontier::resolved_digest(process)
                    .map_err(StoreError::Integrity)?
                    != reference.sha256
            {
                return Err(StoreError::Integrity(
                    "resolved artifact differs from run reference".into(),
                ));
            }
            Ok(bytes)
        })
    }
}
