# Existing-operation queries and resource handover

Status: The finite-operation path `Operation.Reconcile` → P durable query plan → Host read → existing T2/resource handover is connected. It also connects S's actual BT `RequestHandover` requests and lost-response/restart observations. This is not a completion specification for the full recovery procedure or resident operating service.

## Responsibilities and semantics

`Operation.Reconcile` requests confirmation of an existing operation. The RPC response is the `OperationView` at that time. Request acceptance, native completion and resource release are distinct facts. Handling the request itself does not create a new production invocation, cancellation or physical stop.

After checking current Executor authentication, cell negotiation and cell assignment, P preserves a per-operation query plan. Control-session is not yet supported. An already RELEASED operation returns its current state. Requests for the same operation in progress merge into one PENDING plan. An explicit new query after a complete/attention-needed state creates a new plan ID and incremented generation. Events from previous plans remain in the audit ledger.

This query contract differs from replaying Submit's immutable admission receipt. Reconcile returns the latest OperationView without creating a separate result Receipt. S's retained request key tracks network attempts; the unit of P query-plan merging is the operation. The original production command's key/invocation/budget is unchanged.

## Storage and query plans

`rx.internal.reconciliation-request.v1` contains id/operation/cell/host/generation/requested_by/state/issue. The plan and audit event are stored in the same SQLite transaction. States are:

| State | Meaning | Completion condition |
|---|---|---|
| PENDING | Waiting for a read or additional evidence | Does not become complete before result/handover confirmation |
| COMPLETE | P has stored actual RELEASED state | Recheck Work disposition in the current transaction |
| ATTENTION | Authority, record continuity or support scope needs confirmation | Does not automatically become a new production request |

Issues distinguish WaitingDispatch/WaitingResult/WaitingHandover/SourceUnavailable/ContinuityUnproven/PermissionChanged/Unsupported. Repeated updates to the same state do not add events. Plan queries, observation storage and result updates check the current Host account/cell scope and registered session; an old worker cannot update a replaced plan ID.

## Host query sequence

1. If current Work is RELEASED, only finalize the plan as COMPLETE. Even if the process exits between release and plan finalization, this step recovers it on rerun.
2. If no result exists, check for an actual EMIT_ENTERED/DELIVERED outbox. Without dispatch evidence yet, leave WaitingDispatch.
3. Query the existing invocation with `GetReceipt` and `Host.Reconcile`. Apply receipts through the existing handler and evidence batches through the same T2 inbox. A missing receipt or response timeout is not converted into evidence of non-execution.
4. A pending result is WaitingResult. Integrity disputes and UNRESOLVED are ATTENTION.
5. After a terminal result, read the three no-pending/control/support observations from `WatchObservations`. Preserve the original content first, then decide actual handover conditions in the existing `ReleaseResources` in a separate transaction.
6. Change resource holders and Work disposition together only when current operation/cell revisions and all three observations are valid. Subsequent plan COMPLETE confirms the release fact; it is not the cause of release.

The query worker does not call Prepare/Authorize directly. Processing an existing PREPARED receipt may allow the existing dispatcher to continue Authorize only under the original T1's valid permit. A permit sealed by Pause is not permitted again.

## Observation preservation and handover decisions

Preserve the three observations from the authenticated Host's current boot with their original IDs and content. False, insufficient-quality and old observations also remain as query facts. Preservation is not a decision that conditions are satisfied. Different content under the same ID does not overwrite the existing observation; a conflict event and cell-closure restrictions commit first, followed by an integrity error.

Release requires true values for all three kinds, quality/source-age guarantees, maximum age/uncertainty, the same operation/invocation/profile/device session/Host boot, current epoch/scope and cell readiness. Observations predating completion evidence are also rejected. False or stale observations retain the resources while awaiting fresh observations. A success result, SDK response or stop indication alone does not allow handover.

## S request journal and restart

S first stores body and key for visit/node/ReconcileOperation, then calls the RPC after EMIT_ENTERED commits. The response is retained only as `ReconciliationAccepted`. Actual RELEASED in the P snapshot is recorded separately with `ObservedTarget::Released` and a read basis.

Even if P releases after a lost RPC response, S's original PENDING response state is not overwritten with a successful response. Restart `recover` retrieves the same mapping and actual release observation. Repeated BT requests also do not resend an already accepted query as a new RPC each time. Since queries have no CAS change, a path that creates a new key/body based on ABORTED is forbidden.

## Verification scope and remaining connections

- Core: Merging in-progress requests, generation replacement and rejection of old IDs, rejection of COMPLETE before release, preservation of false observations, and restrictions on observation ID conflicts.
- Host integration: Actual mTLS/a separate simulated Host process rejects the first support=false after success, then releases on subsequent evidence. Native effects for the two parts remain two.
- S integration: Actual Linux BT.CPP request → separate S journal/worker → P mTLS query. Checks normal responses, lost responses after commit, subsequent P release and observation by a new S boot. Host completion/handover evidence in this test is an explicit synthetic fixture; its scope is distinguished from the preceding Host integration test.

Queries use at most 4 plans per pass, a 2-second timeout per Host read, 100 ms–5 second backoff and bounded retry memory. Current pending queries are scan-based, and Host queries are serial with the dispatcher pass. Indexing/paging for large ledgers and separate scheduling that bounds Fence delivery delay must be addressed before operating-service implementation. These figures are not real-time response guarantees.

It does not claim that at most 128 consecutive `Host.Reconcile` batches retrieve an entire long backlog. The Host's durable background Evidence.Publish/ack path is also required. Detailed ATTENTION plan queries/UI, operator requery/cancellation/recovery/clearance, control-session, resident loop/part coordinator/liveness remain incomplete. The physical cell remains NOT_COMMISSIONED.
