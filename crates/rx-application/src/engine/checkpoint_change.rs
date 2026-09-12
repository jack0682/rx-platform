use super::executor_peer::executor_scope;
use super::process_transition::{Transition, transition};
use super::*;
use crate::checkpoint_artifact::{self, ExecutorState, RUN_SNAPSHOT, RunSnapshot};

const PROPOSAL: &str = "rx.internal.checkpoint-proposal.v1";
const REPLY: &str = "rx.internal.checkpoint-commit-reply.v1";
const MAX_PROPOSAL_AGE_NS: u64 = 100_000_000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    target: CheckpointTarget,
    session: Id,
    runtime_boot: Id,
    epoch: Counter,
    scopes: BTreeMap<Name, Counter>,
    prepared: PreparedCheckpoint,
}
enum Pending {
    Cached(Box<RunSnapshot>),
    Changed {
        run: Id,
        payload: ArtifactRef,
        scope: Box<RequestScope>,
        fingerprint: Digest,
    },
}

fn equal<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(canonical::bytes(a).map_err(domain_error)? == canonical::bytes(b).map_err(domain_error)?)
}

fn successor(
    tx: &mut dyn Transaction,
    current: &RunSnapshot,
    checkpoint: ProcessCheckpoint,
) -> Result<ExecutorState> {
    let bytes =
        checkpoint_artifact::read_artifact(tx, &current.run.id, &current.checkpoint.payload)?;
    let mut state: ExecutorState = canonical::decode_json(&bytes).map_err(domain_error)?;
    state.revision = current.revision.increment().map_err(domain_error)?;
    state
        .process_checkpoints
        .retain(|c| c.visit != checkpoint.visit);
    state.process_checkpoints.push(checkpoint);
    state.process_checkpoints.sort_by_key(|c| c.visit);
    Ok(state)
}

fn valid_time(proposal: &Proposal, now: &TimePoint) -> bool {
    proposal.prepared.prepared_at.clock_id == now.clock_id
        && proposal.prepared.valid_until.clock_id == now.clock_id
        && now.ticks_ns >= proposal.prepared.prepared_at.ticks_ns
        && now.ticks_ns < proposal.prepared.valid_until.ticks_ns
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Prepare immutable successor state. No run revision, branch, wait or child mapping changes.
    pub fn prepare_process_checkpoint(
        &mut self,
        identity: &Identity,
        target: CheckpointTarget,
    ) -> Result<CheckpointPreparation> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", &target.run, RUN)?;
            let (_, _, cell) = executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            active_run(tx, &cell, &run, identity, meta, &now)?;
            let (_, current): (_, RunSnapshot) = load(tx, "runcheckpoint", &run.id, RUN_SNAPSHOT)?;
            checkpoint_artifact::validate_snapshot(tx, &current)?;
            if current.revision != revision {
                return Err(StoreError::Integrity("current run cut differs".into()));
            }
            let cache_key = key("checkpointproposalcurrent", &target);
            let prior = tx.get(&cache_key)?;
            if let Some(row) = &prior {
                let proposal: Proposal = decode(row, PROPOSAL)?;
                if proposal.session == identity.session
                    && proposal.runtime_boot == meta.runtime_boot
                    && proposal.prepared.expected_revision == revision
                    && proposal.epoch == cell.epoch
                    && proposal.scopes == cell.scope_epochs
                    && valid_time(&proposal, &now)
                {
                    match validate_successor(tx, &current, &cell, &proposal, &now) {
                        Ok(_) => {
                            return Ok(CheckpointPreparation::Ready {
                                proposal: Box::new(proposal.prepared),
                            });
                        }
                        Err(StoreError::Rejected(
                            Reject::StaleRevision | Reject::ConditionUnknown,
                        )) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            let checkpoint = match transition(tx, &run, &cell, &target, &now, &now, None)? {
                Transition::Applied => {
                    return Ok(CheckpointPreparation::AlreadyApplied {
                        run_revision: revision,
                    });
                }
                Transition::Waiting => {
                    return Ok(CheckpointPreparation::Waiting {
                        run_revision: revision,
                    });
                }
                Transition::Changed(value) => value,
            };
            let state = successor(tx, &current, checkpoint)?;
            let checkpoint = checkpoint_artifact::store_state(tx, &state)?;
            let proposal = Proposal {
                target: target.clone(),
                session: identity.session.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                epoch: cell.epoch,
                scopes: cell.scope_epochs,
                prepared: PreparedCheckpoint {
                    expected_revision: revision,
                    checkpoint,
                    valid_until: TimePoint {
                        clock_id: now.clock_id.clone(),
                        ticks_ns: Counter(
                            now.ticks_ns
                                .0
                                .checked_add(MAX_PROPOSAL_AGE_NS)
                                .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                        ),
                    },
                    prepared_at: now,
                },
            };
            let record = doc(PROPOSAL, &proposal)?;
            let artifact_key = key(
                "checkpointproposalartifact",
                (&run.id, proposal.prepared.checkpoint.payload.sha256),
            );
            if let Some(old) = tx.get(&artifact_key)? {
                if old.document != record {
                    return Err(StoreError::Integrity("proposal identity conflict".into()));
                }
            } else {
                tx.put(&artifact_key, None, &record)?;
            }
            tx.put(&cache_key, prior.map(|r| r.revision), &record)?;
            event(tx, "rx.event.checkpoint-proposed.v1", &proposal)?;
            Ok(CheckpointPreparation::Ready {
                proposal: Box::new(proposal.prepared),
            })
        })
    }

    pub fn commit_process_checkpoint(
        &mut self,
        identity: &Identity,
        request_key: &str,
        command: CommitCheckpoint,
    ) -> Result<RunSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact_finalized(
            |tx| {
                let now = clock.now();
                let (revision, run): (_, Run) = load(tx, "run", &command.run, RUN)?;
                let (principal, _, cell) = executor_scope(
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
                    "Workflow.CommitCheckpoint",
                    request_key,
                    &command,
                )?;
                if let Some(reply) = prior(tx, &scope, fingerprint, REPLY)? {
                    return Ok(Pending::Cached(Box::new(reply)));
                }
                check_revision(revision, command.expected_revision)?;
                if command.new_checkpoint.run != run.id
                    || command.new_checkpoint.revision
                        != revision.increment().map_err(domain_error)?
                {
                    return reject(Reject::InvalidInput);
                }
                let (_, proposal): (_, Proposal) = load(
                    tx,
                    "checkpointproposalartifact",
                    (&run.id, command.new_checkpoint.payload.sha256),
                    PROPOSAL,
                )?;
                if proposal.session != identity.session
                    || proposal.runtime_boot != meta.runtime_boot
                    || proposal.epoch != cell.epoch
                    || proposal.scopes != cell.scope_epochs
                {
                    return reject(Reject::ContinuityUnproven);
                }
                if !valid_time(&proposal, &now) {
                    return reject(Reject::Expired);
                }
                if proposal.prepared.expected_revision != revision
                    || !equal(&command.new_checkpoint, &proposal.prepared.checkpoint)?
                {
                    return reject(Reject::InvalidInput);
                }
                active_run(tx, &cell, &run, identity, meta, &now)?;
                let (_, current): (_, RunSnapshot) =
                    load(tx, "runcheckpoint", &run.id, RUN_SNAPSHOT)?;
                checkpoint_artifact::validate_snapshot(tx, &current)?;
                let checkpoint = validate_successor(tx, &current, &cell, &proposal, &now)?;
                super::process::save_checkpoint(tx, &checkpoint)?;
                save(tx, "run", &run.id, Some(revision), RUN, &run)?;
                Ok(Pending::Changed {
                    run: run.id,
                    payload: command.new_checkpoint.payload,
                    scope: Box::new(scope),
                    fingerprint,
                })
            },
            |finalized, pending, _| match pending {
                Pending::Cached(value) => Ok(*value),
                Pending::Changed {
                    run,
                    payload,
                    scope,
                    fingerprint,
                } => {
                    let row = finalized
                        .get(&key("runcheckpoint", &run))?
                        .ok_or(StoreError::Integrity("committed checkpoint missing".into()))?;
                    let snapshot: RunSnapshot = decode(&row, RUN_SNAPSHOT)?;
                    if snapshot.checkpoint.payload != payload {
                        return Err(StoreError::Integrity(
                            "committed artifact differs from proposal".into(),
                        ));
                    }
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

fn validate_successor(
    tx: &mut dyn Transaction,
    current: &RunSnapshot,
    cell: &Cell,
    proposal: &Proposal,
    now: &TimePoint,
) -> Result<ProcessCheckpoint> {
    let bytes = checkpoint_artifact::read_artifact(
        tx,
        &current.run.id,
        &proposal.prepared.checkpoint.payload,
    )?;
    let state: ExecutorState = canonical::decode_json(&bytes).map_err(domain_error)?;
    let seed = state
        .process_checkpoints
        .iter()
        .find(|cp| cp.visit == proposal.target.visit)
        .ok_or(StoreError::Integrity(
            "proposal process checkpoint missing".into(),
        ))?;
    let Transition::Changed(checkpoint) = transition(
        tx,
        &current.run,
        cell,
        &proposal.target,
        now,
        &proposal.prepared.prepared_at,
        Some(seed),
    )?
    else {
        return reject(Reject::StaleRevision);
    };
    let expected = successor(tx, current, checkpoint.clone())?;
    if !equal(&state, &expected)?
        || !equal(
            &proposal.prepared.checkpoint.activations,
            &current.checkpoint.activations,
        )?
    {
        return reject(Reject::StaleRevision);
    }
    Ok(checkpoint)
}
