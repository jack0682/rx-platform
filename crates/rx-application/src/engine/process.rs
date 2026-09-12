use super::process_transition::{Transition, transition};
use super::*;
use rx_process_contract::frontier::{
    self, BranchChoice, OperationProgress, ProgressView, WaitProgress,
};
const CHECKPOINT: &str = "rx.internal.process-checkpoint.v1";

pub(super) fn process_view(
    tx: &mut dyn Transaction,
    run: &Run,
    cell: &Cell,
    visit: Counter,
) -> Result<(ProcessCheckpoint, ProgressView)> {
    let process = cell
        .configuration
        .process
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    let part = process_part(run, visit)?;
    let checkpoint = if let Some(row) = tx.get(&key("checkpoint", (&run.id, visit)))? {
        decode::<ProcessCheckpoint>(&row, CHECKPOINT)?
    } else {
        ProcessCheckpoint {
            run: run.id.clone(),
            visit,
            revision: Counter(1),
            branches: BTreeMap::new(),
            waits: BTreeMap::new(),
            wait_windows: BTreeMap::new(),
            decision_times: BTreeMap::new(),
        }
    };
    let mut operations = BTreeMap::new();
    for row in tx.scan("work/")? {
        let work: Work = decode(&row, WORK)?;
        if work.run == run.id && work.part == part {
            let (_, activation): (_, Activation) =
                load(tx, "activationid", &work.activation, ACTIVATION)?;
            if activation.run != run.id || activation.visit != visit {
                return Err(StoreError::Integrity(
                    "work/checkpoint activation mismatch".into(),
                ));
            }
            if operations
                .insert(
                    activation.node,
                    OperationProgress {
                        intent_digest: work.intent.digest().map_err(domain_error)?,
                        operation: work.operation,
                    },
                )
                .is_some()
            {
                return Err(StoreError::Integrity(
                    "multiple main operations at process node".into(),
                ));
            }
        }
    }
    let view = ProgressView {
        run: run.id.clone(),
        resolved_digest: frontier::resolved_digest(process).map_err(StoreError::Invalid)?,
        complete: true,
        operations,
        branches: checkpoint.branches.clone(),
        waits: checkpoint.waits.clone(),
        cleared_interventions: BTreeMap::new(),
    };
    Ok((checkpoint, view))
}
pub(super) fn process_part(run: &Run, visit: Counter) -> Result<Option<Id>> {
    match run.purpose {
        Some(Purpose::Production) => {
            let index = usize::try_from(
                visit
                    .0
                    .checked_sub(1)
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            )
            .map_err(|_| StoreError::Rejected(Reject::InvalidInput))?;
            Ok(Some(
                run.part_ids
                    .get(index)
                    .cloned()
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            ))
        }
        Some(Purpose::Setup) if visit == Counter(1) => Ok(None),
        _ => reject(Reject::InvalidInput),
    }
}
pub(super) fn eligible_node(
    tx: &mut dyn Transaction,
    run: &Run,
    cell: &Cell,
    node: &Name,
    visit: Counter,
) -> Result<()> {
    let Some(process) = &cell.configuration.process else {
        return Ok(());
    };
    let (_, view) = process_view(tx, run, cell, visit)?;
    let frontier = frontier::plan(process, &view).map_err(StoreError::Integrity)?;
    if !frontier.operations.contains(node) {
        return reject(Reject::ConditionUnknown);
    }
    Ok(())
}
pub(super) fn save_checkpoint(
    tx: &mut dyn Transaction,
    checkpoint: &ProcessCheckpoint,
) -> Result<()> {
    let k = key("checkpoint", (&checkpoint.run, checkpoint.visit));
    let old = tx.get(&k)?;
    let previous = old
        .as_ref()
        .map(|row| decode::<ProcessCheckpoint>(row, CHECKPOINT))
        .transpose()?
        .map(|p| p.revision)
        .unwrap_or(Counter(1));
    check_revision(
        checkpoint.revision,
        previous.increment().map_err(domain_error)?,
    )?;
    tx.put(&k, old.map(|r| r.revision), &doc(CHECKPOINT, checkpoint)?)?;
    event(tx, "rx.event.process-checkpoint-changed.v1", checkpoint)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn start_process_wait(
        &mut self,
        identity: &Identity,
        key_: &str,
        run_id: &Id,
        node: &Name,
        visit: Counter,
        expected_run: Counter,
    ) -> Result<WaitWindow> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            let p = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Executor,
                false,
            )?;
            let (scope, fingerprint) =
                request(meta, &p, "Workflow.StartWait", key_, &(run_id, node, visit))?;
            if let Some(window) = prior(tx, &scope, fingerprint, "rx.internal.wait-window.v1")? {
                return Ok(window);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            active_run(tx, &cell, &run, identity, meta, &now)?;
            let (checkpoint, _) = process_view(tx, &run, &cell, visit)?;
            if let Some(window) = checkpoint.wait_windows.get(node) {
                remember(
                    tx,
                    &scope,
                    fingerprint,
                    "rx.internal.wait-window.v1",
                    window,
                )?;
                return Ok(window.clone());
            }
            check_revision(revision, expected_run)?;
            let target = CheckpointTarget {
                run: run_id.clone(),
                node: node.clone(),
                visit,
                action: CheckpointAction::StartWait,
            };
            let checkpoint = match transition(tx, &run, &cell, &target, &now, &now, None)? {
                Transition::Changed(value) => value,
                Transition::Waiting => return reject(Reject::ConditionUnknown),
                Transition::Applied => {
                    return Err(StoreError::Integrity(
                        "decision changed inside transaction".into(),
                    ));
                }
            };
            let window = checkpoint.wait_windows[node].clone();
            save_checkpoint(tx, &checkpoint)?;
            save(tx, "run", run_id, Some(revision), RUN, &run)?;
            remember(
                tx,
                &scope,
                fingerprint,
                "rx.internal.wait-window.v1",
                &window,
            )?;
            Ok(window)
        })
    }
    /// P-owned reevaluation; neither an executor-supplied clock nor a caller-supplied PASS is used.
    pub fn check_process_wait(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        node: &Name,
        visit: Counter,
        expected_run: Counter,
    ) -> Result<Option<WaitProgress>> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Executor,
                false,
            )?;
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            active_run(tx, &cell, &run, identity, meta, &now)?;
            let (checkpoint, _) = process_view(tx, &run, &cell, visit)?;
            if let Some(result) = checkpoint.waits.get(node) {
                return Ok(Some(result.clone()));
            }
            check_revision(revision, expected_run)?;
            let target = CheckpointTarget {
                run: run_id.clone(),
                node: node.clone(),
                visit,
                action: CheckpointAction::CheckWait,
            };
            let checkpoint = match transition(tx, &run, &cell, &target, &now, &now, None)? {
                Transition::Changed(value) => value,
                Transition::Waiting => return Ok(None),
                Transition::Applied => {
                    return Err(StoreError::Integrity(
                        "decision changed inside transaction".into(),
                    ));
                }
            };
            let result = checkpoint.waits[node].clone();
            save_checkpoint(tx, &checkpoint)?;
            save(tx, "run", run_id, Some(revision), RUN, &run)?;
            Ok(Some(result))
        })
    }
    pub fn process_progress(
        &mut self,
        identity: &Identity,
        run_id: &Id,
        visit: Counter,
    ) -> Result<ProcessProgress> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let checked_at = clock.now();
            let (run_revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize_read(tx, identity, meta, &checked_at, &run.cell)?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            let (checkpoint, view) = process_view(tx, &run, &cell, visit)?;
            let frontier = frontier::plan(
                cell.configuration
                    .process
                    .as_ref()
                    .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?,
                &view,
            )
            .map_err(StoreError::Integrity)?;
            let admission_allowed = match active_run(tx, &cell, &run, identity, meta, &checked_at) {
                Ok(()) => true,
                Err(StoreError::Rejected(_)) => false,
                Err(error) => return Err(error),
            };
            Ok(ProcessProgress {
                checked_at,
                admission_allowed,
                run_revision,
                cell_revision,
                checkpoint,
                view,
                frontier,
            })
        })
    }
    pub fn choose_process_branch(
        &mut self,
        identity: &Identity,
        key_: &str,
        run_id: &Id,
        node: &Name,
        visit: Counter,
        expected_run: Counter,
    ) -> Result<BranchChoice> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
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
                "Workflow.DecideBranch",
                key_,
                &(run_id, node, visit),
            )?;
            if let Some(choice) = prior(tx, &scope, fingerprint, "rx.internal.branch-choice.v1")? {
                return Ok(choice);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            active_run(tx, &cell, &run, identity, meta, &now)?;
            let (checkpoint, _) = process_view(tx, &run, &cell, visit)?;
            if let Some(choice) = checkpoint.branches.get(node) {
                remember(
                    tx,
                    &scope,
                    fingerprint,
                    "rx.internal.branch-choice.v1",
                    choice,
                )?;
                return Ok(choice.clone());
            }
            check_revision(revision, expected_run)?;
            let target = CheckpointTarget {
                run: run_id.clone(),
                node: node.clone(),
                visit,
                action: CheckpointAction::ChooseBranch,
            };
            let checkpoint = match transition(tx, &run, &cell, &target, &now, &now, None)? {
                Transition::Changed(value) => value,
                Transition::Waiting => return reject(Reject::ConditionUnknown),
                Transition::Applied => {
                    return Err(StoreError::Integrity(
                        "decision changed inside transaction".into(),
                    ));
                }
            };
            let choice = checkpoint.branches[node].clone();
            save_checkpoint(tx, &checkpoint)?;
            save(tx, "run", run_id, Some(revision), RUN, &run)?;
            remember(
                tx,
                &scope,
                fingerprint,
                "rx.internal.branch-choice.v1",
                &choice,
            )?;
            Ok(choice)
        })
    }
}
