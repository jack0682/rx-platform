# Platform peer communication boundary

`PlatformIngress` connects an actual mTLS listener to the existing dedicated writer. The resident `rx-platformd` serves it from the configured gRPC listener alongside terminal HTTPS. The separate local development HTTP executable does not automatically activate it. The earlier `EvidenceIngress` name is a compatibility alias. G5.1 language clients use this existing resident surface; base Operation.Submit/Lookup remain deliberately unimplemented and cell admission is mandatory.

## Exposed surface

- `Session.Open`: compare registered Host/Executor/OPERATOR_API leaf certificates and peer ID/role, installation/store generation, release, clock, and base manifest. Check authority against the application's current principal.
- `Cell.Inspect`: read recorded CellContext under the current service session/certificate, cell negotiation, and scope.
- `Cell.Open`: bind the cell manifest and CellDefinition/membership to the same authenticated base session. Reject ambiguous mappings of several cells to the same definition.
- `Workflow.GetRun`: check a registered executor's base/cell negotiation and current cell assignment, then return actual stored RunView/Checkpoint.
- `Cell.BeginPartAttempt`: create a material attempt under the specified run/mandate/budget and current authority, storing budget consumption and response together.
- `Workflow.ResolveActivation`: check request key and run/node/visit/revision, then create the unique activation or retrieve the existing mapping.
- `Cell.SubmitOperation`: bind the complete cell/base envelope, parent, part, and two CAS values for a finite run operation to the same T1; return the initial ADMITTED Receipt.
- `Workflow.CommitCheckpoint`: check current authority, complete request identity, candidate artifact, run CAS, and current conditions; atomically store T3 and the original RunView response.
- Optional `ExecutorPlan.PrepareCheckpoint`: prepare a complete branch/wait state candidate under a separate binding hash. Preparation does not commit process state. Follow the [preparation/commit boundary](../../../rx-application/CHECKPOINT_COMMIT.md).
- `Workflow.PauseRun`: revoke run/cell-closure authorization for explicit executor halt and return the final checkpoint. This is not completed physical cancellation/stopping.
- `Operation.Reconcile`: durably merge a plan to query the existing invocation and return current OperationView. Distinguish acceptance of a query from outcome/resource release. Follow the [query/handover specification](../../../rx-application/RECONCILIATION.md).
- `Operation.Get`: read outcome/knowledge/integrity/resource disposition recorded in P. Reads do not perform native reconciliation/re-execution.
- Optional `ExecutorRead.GetSnapshot/GetArtifact`: read the current process read cut and run-owned checkpoint/resolved bytes under a separate binding hash. Follow the [scope and validation](../../../rx-application/EXECUTION_READ.md).
- Optional `Production.Inspect/CompletePart`: provide consistent run/part/budget inspection from before the first material through completion, and material completion based on P evidence. Admission reuses frozen BeginPartAttempt.
- `Cell.RecordProcedure`: message conversion exists, but human identity binding is not yet connected. Calls adding human roles to service accounts are rejected. Current human reports use direct terminal HTTPS/development HTTP. Follow the [scope](../../../rx-application/PROCEDURE_REPORTS.md).
- `Cell.OpenCase/GetCase`: case intake, hold/withdrawal, and inspection. Do not confuse notification acknowledgment, physical procedures, and clearance. Follow the [current scope](../../../rx-application/INTERVENTION_CASES.md).
- `Evidence.Publish`: original-preserving conversion → T2 in the same writer → durable acknowledgment.
- `Evidence.Get`: retrieve original native evidence after checking producer ownership and current cell access. Reconstruction of originals from incomplete legacy projections is rejected.

CellService/Workflow methods not listed above are explicitly UNIMPLEMENTED. Native submission passes current authorization/condition checks in Cell.SubmitOperation and the existing dispatcher/Host gate. Public base Submit does not bypass cell checks. Do not present this as the complete public P gRPC implementation.

Executor replacement, discarded boots, P restart handling, and read limitations are documented in the [executor session specification](../../../rx-application/EXECUTOR_PEER.md). The presence of a TCP connection or an issued session must not be used as operational readiness or operating authorization.

## Authentication and ordering

Authenticated producer sessions durably bind to journal/peer boot/authentication binding. Reconnection under the same identity converges on the same session regardless of previous response loss. Changes to authentication binding or Host incarnation invalidate the old session and revoke associated existing operational authority. A new session does not mean new operating authorization.

This service identity lifecycle binds to P runtime, Host incarnation, current registration, and roles. It does not reuse the browser's 1-hour session policy. Every TLS request compares registered certificates and session bindings; T2 rechecks current roles/cell access. Dynamic certificate-registry changes and product credential-rotation management remain incomplete.

Publish is a contiguous batch of at most 128 entries/1 MiB. Content differences at the same seq include native_id/native_data. Producer ownership, original content, current outcome, and ledger position enter the same T2; failed T2 emits no ACK. GAP is reported as FAILED_PRECONDITION/GAP with decimal `rx-next-expected` metadata. Sequences are not skipped, and a different journal is not automatically initialized.

## Cursor boundary still to resolve

The producer through_seq in current DurableAck is the actual contiguous inbox commit position, validated through Host retransmission/deduplication tests. platform_cursor now uses **the same transaction cut in a separate control journal**, with the cell extension name `site-cell-control-v1`. Internal audit-event sequences are not mixed in.

Stored payloads are typed application state snapshots. Complete CellJournalRecord/EventType/EntityView mapping remains incomplete, so public Journal.Subscribe/GetSnapshot are not registered. Frozen Journal contract conformance cannot be claimed before completing the required metadata, checkpoint, and wire mapping in the [control journal document](../../../rx-application/CONTROL_JOURNAL.md). Passing acknowledgment tests alone does not promote the whole journal contract to VERIFIED.

Evidence other than currently supported native-result, original artifact export, large-journal retention/gap recovery, and complete P RPC/ledger streams remain future scope.

## Validation

`tools/test_evidence_e2e.sh` builds the test-harness Host executable from the other repository and runs it as a separate process. It sends 131 test journal records contiguously and loses the first data-batch ACK immediately after P commits. It verifies storage of only 131 source slots, lookup of the final original metadata, rejection of unregistered certificates, preservation of the same journal/cursor after Host restart, and invalidation of the old session.

These 131 records are **synthetic uncorrelated evidence**. P does not turn unknown operations into successful work; it blocks affected cells and leaves qualification unregistered. This test produces 0 device effects. The actual native T1–Host–T2–handover path is distinguished through the separate two-simulated-material-attempt test in `tools/test_host_e2e.sh`.

The executor path is validated over actual TLS/SQLite with `cargo test -p rx-api --test executor_ingress`. It checks negotiation of both contracts, certificate/session binding, assigned cells, session replacement, and prohibition of automatic execution, without sending device commands.

The [executor request specification](../../../rx-application/EXECUTOR_REQUESTS.md) describes complete payload/key/CAS and response-loss behavior. After simulated operator/qualification/Host Arm setup, actual mTLS creates part/activation records and confirms them through GetRun. This test has no device submission.

Integration tests for [executor finite submission/result lookup](../../../rx-application/EXECUTOR_SUBMISSION.md) use TLS in both directions and a separate simulated Host. They verify exactly one simulated effect per slot despite lost Submit/Authorize responses.

`Session.Open(OPERATOR_API)`, `Cell.Open`, and `Cell.Inspect` are supported. The operator API authenticates as a transport role and receives neither human-write nor executor authority. `CellContext` uses recorded mode/commissioning/block-creation revisions. Follow the [model, compatibility, and remaining human binding](../../../rx-application/CELL_CONTEXT_AND_OPERATOR_PEER.md).

The earlier draft path that authored human ProcedureRecord using additional Operator/RecoveryLead roles on service accounts is blocked. Human identity in direct terminal HTTPS is handled through [identity binding](../../../rx-application/TERMINAL_IDENTITY.md). OPERATOR_API user delegation over gRPC remains future work.
