# Executor part-attempt and activation requests

Status: connected to actual mTLS RPCs and single-writer/SQLite transactions. This does not mean completion of the full BT worker, device submission or checkpoint mutation APIs.

## Requests and mutation ownership

| RPC | Main inputs | Result recorded by P |
|---|---|---|
| Cell.BeginPartAttempt | Current CellCall, run, mandate, expected budget revision, optional cell revision | PartAttempt ID/ordinal/revision, run-owned budget consumption and run revision, journal/recovery state |
| Workflow.ResolveActivation | Current CallContext, key, run, node, visit, expected run revision | Unique activation or existing mapping; run revision and recovery state for a new allocation |

The RPC adapter checks message format, revision placement and required keys, then forwards a typed command to the writer. Roles, terminals or approval bools in JSON/Protobuf do not create an execution identity. Time in the internal `ProcessingContext` is P time obtained when the writer processes the request.

`executor_requests` for public requests handles request envelopes and durable results. Actual part creation/budget consumption and activation creation/eligibility use the same transition functions as existing workflow paths. There is no separate SQL connection, operation-number allocator or second business state machine.

## Check and storage order

1. Check current P session/account Executor role, account cell scope, CellDefinition negotiation and the cell's executor assignment. A historical key cannot bypass this check after a new boot or role revocation.
2. Compare the request key and fingerprint of the entire typed business payload. If the same request is already applied, return its stored response from that time.
3. A new BeginPartAttempt checks the run/cell relationship, explicit mandate, optional cell revision, current run/mandate/qualification/conditions and budget revision/remaining budget.
4. A new ResolveActivation first recovers any existing run/node/visit mapping. A new mapping requires checks of run revision/current execution authority/actual part ordinal/process frontier.
5. Core changes, request result, journal, current projection and checkpoint artifact commit in the same repository transaction. Errors roll them back together.

Repeating the same key/body after a lost response does not consume budget again or create a new activation. Obtaining a new revision from a read is not a reason to change the body for the same key. Changed mandate/visit/revision is `KEY_CONFLICT`, returned as gRPC `ALREADY_EXISTS`. New intent requires a new key.

An existing `(run,node,visit)` returns its existing activation even with a new key. Recovering an existing mapping is distinct from CAS for a new allocation, so a read-like repeat request using the revision before that mapping existed does not create a new ID. Current identity/cell access checks still apply.

CallContext call ID and the authentication session itself are excluded from the business fingerprint. The key belongs to installation/client namespace/method. Business inputs and CAS values are included in the fingerprint. If the existing simple trusted-composition helper and public typed request have different payloads, a key is not reused across them. Different payloads are not treated as the same request; any added public BFF must use this typed path.

## Revisions and responses

The BeginPartAttempt response stores the actual PartAttempt record revision from that commit. Later part disposition is not read to attach only a new revision to a historical response. `material_id` is absent because no material identity is bound into the model yet; an arbitrary material UUID is not issued and presented as an identified physical item.

ActivationView slot/operation/intent bindings are created in the same transaction after checking Work and run/part/cell relationships. A historical key's stored response is distinct from the latest GetRun. Child mappings created later are recovered from GetRun's new checkpoint.

The existing frozen codec is used instead of default ProtoJSON numeric representation. Positive visit/budget/run revisions are required; CellCall's base expected_revision is not accepted in duplicate with object-specific expected_* fields.

## Validation scope

- Consistent part count/budget/activation/checkpoint on rollback immediately before actual persistence and lost response immediately after commit.
- Conflicts between the same key and changed mandate/budget/visit; existing activation recovery with a new key.
- Even after budget exhaustion, the same Begin request returns the original part without further consumption.
- After role revocation, cached part/activation responses cannot bypass authorization.
- Actual TLS socket/single writer/SQLite connection through Session → Cell negotiation → simulated operator start → BeginPartAttempt → ResolveActivation → GetRun.

The transport tests' operator terminal, qualification, Host readiness/Arm acknowledgment are explicit simulation fixtures. These tests do not create Work or invoke a native device. Actual device delivery is a separate Host E2E scope.

## Next connections

The [Cell.SubmitOperation finite-operation path](EXECUTOR_SUBMISSION.md) is enabled. It connects run/activation/slot/part and parent mandate, nested CallContext and two-object CAS in the same T1, returning an [initial admission receipt](ADMISSION_RECEIPT.md) at the actual journal location. Continuous-control/recovery submission and the process-control connections below are follow-up work.

P branch/wait decisions, Workflow.CommitCheckpoint release schema/CAS, E artifact acquisition, C++ Frame/finite-operation/Pause/handover workers and pending-key recovery were connected in later stages. S branch/wait workers are connected; complete recovery remains outstanding. The behavior of these two requests is not presented as complete executor execution/recovery/operating qualification.
