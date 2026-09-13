# Recovery data and current execution-state reads

Status: connected to shared DTOs, P's current read cut, a separate S client, and C++ Frame inputs/clock checks. S's durable requests and finite-operation worker are also connected; all request types/daemon/automatic recovery remain follow-up work.

## Checkpoint and live snapshot

A Checkpoint is historical recovery data stored at a particular Run revision. Operation results, resource state and observation age can change afterward, so its EXECUTING state alone cannot be used as current permission to issue requests.

`Engine::execution_snapshot` reads current executor access/definition negotiation, Run/cell epoch/scope, current process progress, checkpoint and journal position in one transaction. The returned position is the actual control cut of that transaction. It includes request eligibility and the domain reason for denial. Storage-integrity/read failures are not hidden as a simple 'not allowed'.

Conditions can age without a new packet, so active_run reevaluates maintained conditions at actual processing time. New activation/part/operation admissions also use this shared check. It does not replace independent local protection for equipment already running or supervisor aging/liveness monitoring.

## Shared data and responsibilities

Run/Purpose/RunState, ProcessCheckpoint/WaitWindow and checkpoint/activation/slot DTOs moved to ROS/BT/I/O-independent `rx-process-contract::execution`. Existing checkpoint schema/fields/canonical-byte semantics are preserved. Data models are shared in the S SDK; P's Engine/transactions/authority writer are not exported.

`rx.execution-snapshot.v1` is the current executor-read schema. It includes installation/store generation/runtime boot, caller session, control sequence, cell definition/envelope/revision/epoch/scopes, P clock bounds, run/checkpoint, requested visit, resolved ArtifactRef, current process progress and admission flag/reason.

The shared validator checks schema/clock bounds, run/checkpoint/visit, actual resolved bytes/hash/schema/size, journal/scope position, the relationship of current authority flags to the Run, checkpoint/progress decision consistency, complete activation/slot/operation bindings and intent digests. Recovery data is not replaced with caller-assembled operation results.

## Optional read binding

Current process-specific progress is not added as arbitrary fields to a frozen RunView. A separate [`rx.executor.v1` read binding](../../spec/executor/v1/README.md) requires the exact binding manifest hash on every call.

- GetSnapshot: returns a read cut of the currently verified graph/run/visit as typed canonical bytes and an exact ArtifactRef.
- GetArtifact: returns only run-owned checkpoints or the resolved process matching the current run. No arbitrary path/URL, general blob writes or proposal submission.

Existing mandatory base/cell peer negotiation rules and mutations are preserved. These additional read APIs do not provide a bypass replacing Workflow.CommitCheckpoint or Cell.SubmitOperation. Additional protobuf/DTO/validator/semantic documents are pinned in a separate manifest and compared by Cargo builds and checking tools. The eight frozen normative files and manifests are unchanged.

Maximum payload is 1,000,000 bytes; total gRPC size is 1 MiB. Oversized data is not truncated to return a partial snapshot. Current source collection uses scans; large-scale paging/indexing/retention remains incomplete.

## Time and the C++ boundary

A P snapshot is valid for at most 100 ms and never beyond session expiry. This is the maximum age of a current-state read, not permission for physical action. New T1 transactions recheck current authority, conditions and resources.

The S client checks certificates, both base negotiations, read binding, data hash/size/schema, the same installation/store generation/runtime/cell/session/journal position, the actual resolved process and all state bindings. It compares P read time with trusted same-host clock bounds before/after the request. A separate request-send Instant bound is also applied.

The Linux adapter uses kernel boot ID and CLOCK_BOOTTIME. Currentness is not judged only by a local steady deadline that may stop during host suspend. A C++ Frame includes both original source clock ID/start/expiry and a short remaining local lifetime. Context rechecks the source clock at publish/tick/queue handoff and emits no requests on mismatch, expiry or clock error.

The C++ decoder rejects duplicate/extra JSON keys, uint64 sent as numbers, overflow, unknown enums, invalid IDs, duplicate eligible nodes and invalid time bounds. It does not automatically adopt a new epoch/session/digest into an existing Context. Malformed Frames follow existing pause rules. Test-only in-process synthetic Frames are distinguished from actual IPC packets' source-clock obligations.

## Validation and next stages

- Compare branch decisions, journal cut, run-owned artifacts and fresh/aged maintained conditions against actual P data.
- Verify that a new S process connecting through mTLS revokes the existing E session/Run authority, recovers previous operations/slots and has new admission=false.
- Feed that client's Frame into the actual Linux C++ BT engine and verify no unauthorized reexecution/success handling.
- Counterexample where the source clock expires while the local steady deadline remains; rejection of automatic epoch adoption and duplicate/unknown/overflow inputs.
- Execute C++ and Rust CLOCK_BOOTTIME adapters on actual Linux. Native Linux Rust client tests ran offline with the pinned toolchain and existing verification image.

Clocks/operator/qualification/device state are explicit simulation fixtures. The pending-key journal, finite-operation/Pause/handover workers and P branch/wait/CommitCheckpoint were connected in subsequent stages. S branch/wait workers are connected; the product BT daemon/Frame loop, complete recovery/liveness, two product images and installation/restoration/acceptance remain incomplete.

## Historical Runs after configuration replacement

Use [per-operation immutable configuration](PROCESS_APPLY.md). Read resolved/process from the configuration at Run creation, while current cell revision/epoch/scopes and admission come from the current cell. A completed old process is not interpreted using the latest recipe. New execution admission requires complete configuration agreement.
