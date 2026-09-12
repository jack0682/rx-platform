use super::*;
use crate::projection::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn run_checkpoint(
        &mut self,
        identity: &Identity,
        run_id: &Id,
    ) -> Result<crate::checkpoint_artifact::RunSnapshot> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository
            .transact(|tx| read_run_checkpoint(tx, identity, meta, &clock.now(), run_id))
    }

    pub fn checkpoint_artifact(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        reference: &ArtifactRef,
    ) -> Result<Vec<u8>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize_read(tx, identity, meta, &clock.now(), &run.cell)?;
            crate::checkpoint_artifact::read_artifact(tx, run_id, reference)
        })
    }
    pub fn session_profile(&mut self, identity: &Identity) -> Result<UserProfile> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = authorize_identity(tx, identity, meta, &clock.now())?;
            profile(tx, principal, identity)
        })
    }

    /// All displayed entities and the caller's access are read at one transaction cut.
    /// Control journal sequence numbers and other cells are deliberately absent.
    pub fn overview(&mut self, identity: &Identity) -> Result<Overview> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize_identity(tx, identity, meta, &now)?;
            let mut cells = BTreeMap::new();
            for record in tx.scan("cell/")? {
                let cell: Cell = decode(&record, CELL)?;
                let config = &cell.configuration;
                if !principal.cells.contains(&config.id) {
                    continue;
                }
                cells.insert(
                    config.id.clone(),
                    CellOverview {
                        diagnostics: diagnostics::read(tx, &cell, meta, &now)?,
                        cell: Versioned {
                            revision: record.revision,
                            value: CellSummary {
                                id: config.id.clone(),
                                mode: cell.mode,
                                commissioning: cell.commissioning,
                                environment: config.environment,
                                epoch: cell.epoch,
                                definition: config.definition.clone(),
                                envelope: config.envelope.clone(),
                                recipe: config.recipe.clone(),
                                site_config_digest: config.site_config_digest,
                                hosts: config.hosts.clone(),
                                qualification: cell.qualification,
                                blocks: cell.blocks,
                                open_cases: cell.open_cases,
                            },
                        },
                        runs: Vec::new(),
                        work: Vec::new(),
                        runs_truncated: false,
                        work_truncated: false,
                    },
                );
            }
            for record in tx.scan("run/")? {
                let run: Run = decode(&record, RUN)?;
                if let Some(view) = cells.get_mut(&run.cell) {
                    view.runs.push(Versioned {
                        revision: record.revision,
                        value: run,
                    });
                }
            }
            for record in tx.scan("work/")? {
                let work: Work = decode(&record, WORK)?;
                if let Some(view) = cells.get_mut(&work.cell) {
                    view.work.push(WorkSummary {
                        cell: work.cell,
                        run: work.run,
                        part: work.part,
                        host: work.host,
                        operation: work.operation,
                    });
                }
            }
            for view in cells.values_mut() {
                // Bounded response pages; UUID order is a stable presentation order, not a clock.
                view.runs.sort_by(|a, b| b.value.id.cmp(&a.value.id));
                view.work
                    .sort_by(|a, b| b.operation.id().cmp(a.operation.id()));
                view.runs_truncated = view.runs.len() > 50;
                view.work_truncated = view.work.len() > 100;
                view.runs.truncate(50);
                view.work.truncate(100);
            }
            Ok(Overview {
                snapshot_id: id(),
                installation: meta.clone(),
                observed_at: now,
                user: profile(tx, principal, identity)?,
                cells: cells.into_values().collect(),
            })
        })
    }

    /// Logout revokes the session, not an already committed human-authorized production mandate.
    /// Service identities must use their explicit disconnect/reconciliation lifecycle.
    pub fn end_user_session(&mut self, identity: &Identity) -> Result<()> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = authorize_identity(tx, identity, meta, &clock.now())?;
            if principal.roles.contains(&Role::Host)
                || principal.roles.contains(&Role::Executor)
                || principal.roles.contains(&Role::OperatorApi)
            {
                return reject(Reject::Forbidden);
            }
            let (revision, mut session): (_, Session) =
                load(tx, "session", &identity.session, SESSION)?;
            session.active = false;
            save(
                tx,
                "session",
                &session.id,
                Some(revision),
                SESSION,
                &session,
            )?;
            event(tx, "rx.event.user-session-ended.v1", &session.id)
        })
    }
}

fn profile(
    tx: &mut dyn Transaction,
    principal: Principal,
    identity: &Identity,
) -> Result<UserProfile> {
    let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)?;
    Ok(UserProfile {
        principal: principal.id,
        terminal: session.terminal.as_ref().map(|t| t.id.clone()),
        roles: principal.roles,
        cells: principal.cells,
        expires_at: session.expires_at,
    })
}

pub(super) fn read_run_checkpoint(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    run_id: &Id,
) -> Result<crate::checkpoint_artifact::RunSnapshot> {
    let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
    authorize_read(tx, identity, meta, now, &run.cell)?;
    let (_, snapshot): (_, crate::checkpoint_artifact::RunSnapshot) = load(
        tx,
        "runcheckpoint",
        run_id,
        crate::checkpoint_artifact::RUN_SNAPSHOT,
    )?;
    if snapshot.revision != revision
        || canonical::bytes(&snapshot.run).map_err(domain_error)?
            != canonical::bytes(&run).map_err(domain_error)?
    {
        return Err(StoreError::Integrity("run checkpoint cut differs".into()));
    }
    crate::checkpoint_artifact::validate_snapshot(tx, &snapshot)?;
    Ok(snapshot)
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Immutable T1 acknowledgment. It does not describe later Host delivery or native completion.
    pub fn admission_receipt(
        &mut self,
        identity: &Identity,
        operation: &Id,
    ) -> Result<AdmissionReceipt> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            authorize_read(tx, identity, meta, &clock.now(), &work.cell)?;
            let (_, receipt): (_, AdmissionReceipt) = load(
                tx,
                "admissionreceipt",
                operation,
                crate::control_journal::ADMISSION_SCHEMA,
            )
            .map_err(|e| missing_as(e, Reject::ContinuityUnproven))?;
            if receipt.operation != *operation
                || receipt.intent_digest != work.intent.digest().map_err(domain_error)?
                || receipt.intent_digest != work.operation.intent_digest()
                || receipt.operation_revision.0 == 0
                || receipt.operation_revision > work.operation.revision()
                || receipt.sequence.0 == 0
            {
                return Err(StoreError::Integrity(
                    "admission receipt identity differs".into(),
                ));
            }
            Ok(receipt)
        })
    }
}
