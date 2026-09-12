use super::executor_peer::executor_scope;
use super::*;
use crate::checkpoint_artifact::{RUN_SNAPSHOT, RunSnapshot};
const REPLY: &str = "rx.internal.pause-run-reply.v1";
enum Pending {
    Cached(Box<RunSnapshot>),
    Changed {
        run: Id,
        scope: RequestScope,
        fingerprint: Digest,
    },
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn pause_executor_run(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: PauseRunRequest,
    ) -> Result<RunSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact_finalized(
            |tx| {
                let now = clock.now();
                let (revision, mut run): (_, Run) = load(tx, "run", &command.run, RUN)?;
                let (principal, _, _) = executor_scope(
                    tx,
                    ProcessingContext {
                        identity,
                        meta,
                        now: &now,
                    },
                    &run.cell,
                )?;
                let (scope, fingerprint) =
                    request(meta, &principal, "Workflow.PauseRun", request_key, &command)?;
                if let Some(saved) = prior(tx, &scope, fingerprint, REPLY)? {
                    return Ok(Pending::Cached(Box::new(saved)));
                }
                check_revision(revision, command.expected_run)?;
                match run.state {
                    RunState::Executing => {
                        if run.executor_session.as_ref() != Some(&identity.session) {
                            return reject(Reject::MandateRevoked);
                        }
                        let affected = pause_closure(tx, &run.cell, &run.id)?;
                        event(
                            tx,
                            "rx.event.executor-pause-requested.v1",
                            &(&principal.id, &run.id, &affected),
                        )?;
                    }
                    RunState::Prepared if run.pending_attempt.is_some() => {
                        let affected = pause_closure(tx, &run.cell, &run.id)?;
                        event(
                            tx,
                            "rx.event.executor-pause-requested.v1",
                            &(&principal.id, &run.id, &affected),
                        )?;
                    }
                    RunState::Prepared => {
                        // No mandate, permit or in-flight Arm exists for this prepared run.
                        run.state = RunState::Paused;
                        save(tx, "run", &run.id, Some(revision), RUN, &run)?;
                        event(
                            tx,
                            "rx.event.executor-pause-requested.v1",
                            &(&principal.id, &run.id),
                        )?;
                    }
                    RunState::Paused
                    | RunState::RecoveryRequired
                    | RunState::Completed
                    | RunState::Abandoned => {}
                }
                Ok(Pending::Changed {
                    run: run.id,
                    scope,
                    fingerprint,
                })
            },
            |finalized, pending, _| match pending {
                Pending::Cached(value) => Ok(*value),
                Pending::Changed {
                    run,
                    scope,
                    fingerprint,
                } => {
                    let row = finalized
                        .get(&key("runcheckpoint", &run))?
                        .ok_or(StoreError::Integrity("pause checkpoint missing".into()))?;
                    let snapshot: RunSnapshot = decode(&row, RUN_SNAPSHOT)?;
                    finalized.remember(
                        &scope,
                        &SavedRequest {
                            fingerprint,
                            result: doc(REPLY, &snapshot)?,
                        },
                    )?;
                    Ok(snapshot)
                }
            },
        )
    }
}
