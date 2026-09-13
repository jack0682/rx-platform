# P process decisions and checkpoints

When `CellConfiguration.process` is present, P directly verifies the shared resolved process. It compares the Recipe ArtifactRef digest/schema/size against actual graph bytes, and the operation node set against StepBinding Host/Intent. In graph mode, predecessor lists are empty to avoid interpreting a separate predecessor list and graph rules simultaneously.

The existing process=None finite configuration is a compatibility path and does not provide branch/wait functionality. Graph configurations are not reduced to simple lists to bypass conditions.

## Authority and storage

ProcessCheckpoint belongs to a run+visit and stores branch decisions, wait windows/results and decision times. A PRODUCTION visit is the actual part ordinal; SETUP uses 1. The compiler expands Repeat/Call into unique node identities so that the same source step does not collide across different repetition positions.

- `choose_process_branch`: Checks current executor/run authority and revision, and the current frontier. Evaluates actual stored facts/quality/source generation/age and records a selection only for PASS/FAIL. UNKNOWN does not select. A selection for the same node is not changed even if facts later change. Actuation conditions for a new operation are rechecked separately at T1.
- `start_process_wait`: Pins start/deadline using P's time. A request with a different key retrieves the same window without extending the deadline.
- `check_process_wait`: P evaluates current conditions and the deadline. The result is recorded with decision ID/evidence/time. It does not accept caller time or a PASS bool. Timeout is not native stopping/operation completion.
- `process_progress`: Builds a complete ProgressView from P's stored state and computes the frontier. admission_allowed is separate from current run/executor authority; readability does not grant execution permission.

Decisions, checkpoint/run revisions and idempotency results share the same transaction. The control ledger also captures checkpoint changes. Explicit intervention continuation is not currently provided, so that frontier remains blocked. There is no P API that passes by accepting an arbitrary clearance ID.

## Activation and completion

Both ResolveActivation and T1 of a new Submit slot check that the operation node is permitted by P's frontier. Activation cannot skip an unselected branch, incomplete wait or predecessor. Retrieving an existing slot/key is distinguished from new admission.

Part completion verifies completion of the selected graph and each operation's result/handover. It does not require operations for unselected branches to be created. Checkpoints/decisions survive P restart, but the run becomes RECOVERY_REQUIRED and admission_allowed is false. A new session does not revive past execution authority.

## Current external connections

This API is connected to actual application transactions and Runtime typed Commands. [Content-addressed checkpoint artifacts and RunView reads](CHECKPOINT_ARTIFACT.md) are connected to the same transaction/local API. Payload/CAS and actual mTLS for [candidate preparation and external Workflow.CommitCheckpoint](CHECKPOINT_COMMIT.md) are connected. Internal paths and candidate finalization use the same state-transition/checkpoint-storage functions. S Frame, finite-operation/Pause/handover workers are connected, as are branch/wait workers and observation restoration. Complete CellJournal mapping and explicit restart procedures remain incomplete.

Progress composition and correlation queries still use entity scans. Large-scale indexing/performance validation and a complete recovery/restart/clearance model remain outstanding. This implementation does not mean complete delivery or verified operation of physical equipment.

## Verification

P tests check rejection of unselected-branch activation, retrieval of the same result after a lost decision response, preservation of the existing selection after condition changes, rejection of wait-deadline resets and actual P timeout/success, rejection of recipe-binding replacement, decision preservation/authority revocation after restart, and part completion using only the selected branch and handovers. Equipment results and handovers use explicit simulation fixtures.
