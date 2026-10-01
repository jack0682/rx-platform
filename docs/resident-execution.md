# Owner-approved resident execution

This optional path runs a P-owned component through the existing Supervisor,
registration journal and OS requirement enforcement. It is process lifecycle,
not permission to perform business work or evidence of physical completion.

1. On the same Linux boot/filesystem domain, provision a dedicated service
   certificate mapped in P to an active principal whose sole role is SUPERVISOR.
   It cannot use ordinary user, Observer, Host or Cell Executor sessions.
2. Run the paired S command `rx-solutionsd catalog CONFIG` with a runtime
   configuration containing schema `rx.resident-execution-runtime.v1` and a
   `state_subdirectory`. This verifies `/opt/rx`, records its release floor and
   emits actual registry/release/program enrollment data. No connection or
   assignment is required for this command and no process is started.
3. Review that data and add it under P configuration
   `resident_supervisors.<principal>`. Enroll its certificate through the existing
   `grpc.allowed_certificates` map. Empty enrollment is the default. Program
   catalog digests include author-owned effect, arguments, readiness and requirements.
4. The owner registers components with those exact catalog references, then POSTs
   a request-key envelope to `/api/v1/resident-executions`. Its command names the
   Supervisor, environment, profiles and a map of selections. Each selection pins
   component/expected_revision, site parameters, depends_on, startup_timeout_ms
   and shutdown_timeout_ms. P allocates the run and instance IDs.
5. Add the returned assignment ID and an execution connection file to S CONFIG,
   then run `rx-solutionsd platform-run CONFIG`. It submits concrete verification
   from the actual release/catalog and waits for owner approval. The connection
   schema is `rx.resident-execution-connection.v1`; fields are endpoint, server_name,
   ca, certificate, private_key, principal, installation, store_generation,
   shared_clock_id and release_digest. The clock is the actual shared Linux boot
   clock, not a configurable substitute for time. The last release_digest is the
   expected P installation release context; accepted S content releases are the
   separately enrolled program-release digests.
6. GET `/api/v1/resident-execution?id=...`, review the preparation and POST
   `/api/v1/resident-executions/approve` with assignment, expected_revision,
   preparation_digest and start_window_ms (100..30000). Keep the request key/body
   for exact recovery if the response is lost. Stop through
   `/api/v1/resident-executions/stop` with assignment/expected_revision, or locally
   signal the running S command. Neither operation proves exit until observed.

The execution session is bound to the canonical registry path. Imported components
also require their original source cut and completed S acknowledgement; use that
same source registry, not a fresh directory. Local declarations remain fenced.
Old operational writers are refused by reader schema 9. Source-only seals remain
7, intake-only stores 8 and ordinary stores 6; promotions never lower a reader floor.

A live grant is private, finite and single use. S compares it to the actual local
plan/catalog and persists consumption before OS invocation. A different local
manager cannot adopt or overwrite the original execution. Automatic restart is
zero; the same original assignment cannot become a new attempt after restart.
Each full set of per-process requirements is enforced by the existing backend.
This does not establish all-or-none process creation for a multi-selection plan
or complete framework-wide logical/physical resource reservation.

P holds a logical component claim while an issued attempt is unresolved. Changes
and retirement of that declaration require stopping/reconciling the assignment
first. Current P/S incarnation, enrollment, role and expected revisions are
rechecked at the appropriate writer calls. TTL alone cannot release a claim.
A View's phase reflects its last accepted source observation; peer_current is a
protocol-context comparison, not process liveness. Declaration snapshots and
content verification checkpoints are not continuous readiness or work permission.

A separate worker persists and delivers execution observations and polls P stop
requests. It does not call or block the local process manager. During an outage,
local stop can finish while P retains claims and S retains the original pending
report. That situation needs explicit reconciliation; this revision does not
silently resume an active-unknown run or issue a new grant.

The current command is a development implementation. See the
[contract](https://github.com/jack0682/rx_docs/blob/develop/docs/contracts/resident-execution/v1/README.md)
and `tools/test_resident_execution.py --help` for the private Linux verification
workflow. A new paired installer/release, active-unknown recovery, broader
content extensibility, live replacement, work readiness and physical qualification
remain separate completion requirements.
