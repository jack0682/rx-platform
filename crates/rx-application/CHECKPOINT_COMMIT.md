# Preparing branch/wait candidates and committing checkpoints

Status: P internal transitions, Runtime typed commands, optional PrepareCheckpoint and the frozen contract's Workflow.CommitCheckpoint are implemented. S's branch/wait workers, durable requests and observation recovery were connected in subsequent stages. Also read the [executor validation scope](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-executor/DECISIONS_AND_RECOVERY.md).

## Use complete state candidates

The frozen ChangeCheckpoint carries a state artifact and existing activation/slot bindings. It is not repurposed as an arbitrary command envelope. PrepareCheckpoint creates the complete next `rx.executor-state.v1` candidate from P's current state, and the executor proposes that Checkpoint unchanged to CommitCheckpoint.

The candidate preserves run metadata/purpose/budget/session/part ID and all activation/slot bindings, proposing only a branch selection, wait start or wait result for one visit/node. The new revision is current+1. The candidate artifact is preserved by content address and can be read through the run-owned artifact path, but does not replace the current RunView. Readable candidate/historical artifacts are not current execution permission.

## Preparation and commit responsibilities

| Stage | What P checks | What changes atomically |
|---|---|---|
| Preparation | Current executor authority/cell assignment, run/visit/node, frontier, P observations and time | Candidate artifact, ownership reference and preparation record. Run/process decisions/operations remain unchanged |
| Waiting | Observation is UNKNOWN or the wait condition is not yet satisfied | Return WAITING without creating a decision/candidate |
| Already committed | The branch/window/result is already stored | Return ALREADY_APPLIED. The executor reads the snapshot again |
| Commit | Current authentication → complete request key/body → run CAS/candidate ownership/lifetime/generation → current operating eligibility → state recomputation | Record process checkpoint, Run revision, current artifact/control events and original response in the same commit |

The existing internal choose/start/check paths and candidate preparation/commit use the same transition functions in `process_transition.rs`. Process checkpoint persistence also uses one function that checks the next revision. Transport handlers contain no separate condition judgments or SQL.

## Do not apply stale decisions unchanged

A candidate is bound to P runtime boot, executor session, cell epoch/scope, base revision, P preparation time and a maximum lifetime of 100ms. At commit, state is recomputed from current facts, and the complete artifact bytes and existing ID bindings are compared. Even with an unchanged run revision, changed observation evidence can cause rejection. Candidates affected by authority revocation, a new session or a new boot do not revive operation.

Callers cannot freely submit a branch bool or completion state. No upload API is provided. The exact reference and schema of the artifact P prepared must be used, and its state must match current evaluation. Incorrect schema/size/revision, another run, or added/deleted/replaced activations/slots are not permitted.

## Wait timing

A wait-start candidate's started_at and deadline are set from the initial P preparation time. A successful commit fixes that window without extending it by network delay. Once committed, a window does not extend even if restarted with another key. Discarding an expired candidate and preparing again replaces an attempt that has not yet been committed.

Even with a condition-satisfied candidate, satisfaction is not applied if the actual commit time is at or beyond the deadline. A new preparation proposes TIMED_OUT. Timeout is unrelated to native stop, operation completion or resource handover. A commit does not proceed if clock continuity cannot be established.

## Lost responses and preservation

A request for an already committed change with the same key/body returns the original RunView without fabricating a response from the current revision. Current role and cell access are checked before replay as well. Failure before persistence does not change the process or revision. A lost response after persistence does not undo the fact.

When a prepared artifact is accepted as the current checkpoint, the transaction completes only if the digest of the automatically generated final artifact also matches. The final response is not replaced with the candidate itself or another artifact of current state.

## Transport and validation

PrepareCheckpoint requires a [separate specification and binding hash](../../spec/executor-plan/v1/README.md). The existing eight base/cell normative files and executor-read binding are preserved. CommitCheckpoint uses the existing frozen protobuf and body expected_revision rules unchanged.

Core tests verify non-application of candidates, preparation/commit rollback and lost responses, same-key/full-body conflicts, current-role checks, artifact/mapping tampering, observation changes, candidate expiry, wait deadline boundaries and window preservation. Actual mTLS tests verify branch and wait preparation → commit → artifact read, and recovery after a lost commit response. Existing finite-operation/Pause/handover/recovery paths are also regression-tested.

S's durable branch/wait requests and result observations, and re-preparation after structured checkpoint rejection, are connected. Remaining features include the resident BT loop, intervention/clearance and recovery, candidate cleanup/capacity/indexing for long runs, the complete operator UI and product deployment. The current single-state artifact size limit and scan cost are not considered resolved. Clocks and observations are simulated fixtures, and the physical cell is NOT_COMMISSIONED.
