# RX execution read binding v1 — implementation draft

This optional release binding supplements frozen base/cell contracts with typed, read-only process data. It does not replace Workflow/Cell mutations, grant operating authority, accept caller state assertions or expose P's database.

A call requires the registered executor's current mTLS session, base/cell negotiation, current role/cell membership and executor assignment. Each request carries the exact binding manifest hash. A base/cell-only peer remains usable for its existing methods; use of this optional service explicitly opts into this additional binding.

- GetSnapshot(context, run, positive visit, binding_hash) returns one P read cut in canonical rx.execution-snapshot.v1 bytes with an exact ArtifactRef. The snapshot contains current run/checkpoint identity, process progress, scope epochs, source position, P clock bounds and whether admission requests are currently allowed. The latter is not a dispatch permit or a physical safety guarantee.
- GetArtifact(context, run, ArtifactRef, binding_hash) returns only that run's owned checkpoint or its matching configured resolved process. It accepts no path or URL, grants no native access and never writes a proposal or commits a checkpoint.

The payload is closed typed JSON from the shared rx-process-contract DTO. Consumers verify schema/hash/size and structural, identity, plan and state relationships before use. Unknown fields, invalid uint64 encodings, malformed snapshots and partial/different-plan views are rejected. Raw bytes must not become an unvalidated free-form task payload.

Snapshot validity is bounded to at most100 ms from the P read time and by the authenticated session expiry. This is a stale-view bound for planning/request generation, not permission to bypass T1 or maintained-condition checks. A client verifies P read time against trusted same-host clock samples taken before/after the request, and checks the absolute source expiry. It also uses request-send monotonic time as a conservative local round-trip bound. Both bounds must remain valid; a local steady-clock deadline alone must not hide a suspend interval. Linux clock adapters use CLOCK_BOOTTIME with the kernel boot ID. The C++ IPC frame carries the source clock bounds and checks them again at publication, tick and queue handoff. It must not automatically adopt a new epoch/session and resume work.

Each payload is at most1,000,000 bytes; the gRPC envelope is at most1 MiB. Oversized views fail instead of being truncated. Indexed/paged large-run reads and artifact retention remain incomplete. Artifact content hashing is not a signature or proof of current operating authority; endpoint authentication and current P checks remain required.

Read-only snapshots preserve branch/wait decisions and all result/knowledge/integrity/disposition axes. If a maintained condition has aged out, current admission is false even when the historical Run state is EXECUTING. Aging detection/physical local protection and supervisor liveness still need their separate mechanisms.

The binding manifest pins this document, the explicit protobuf and shared DTO source. Existing frozen base/cell normative documents and their manifest hashes are unchanged. This binding and its tests must join the product release manifest before deployment qualification; matching hashes alone do not prove hardware behavior.
