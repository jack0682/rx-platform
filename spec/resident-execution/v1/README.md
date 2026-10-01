# Resident execution binding, revision 1 (implementation in progress)

This contract connects P-owned component declarations to S-owned process creation.
It does not turn a registration, reporting scope or transfer acknowledgement into
execution permission. It is separate from frozen Host/Executor/cell bindings.

P enrolls an exact Supervisor-only service principal, permitted release digests
and complete catalog program digests/effect classes through trusted startup
configuration. S must verify actual release bytes and construct programs through
the existing trusted release catalog. P accepts these as authenticated enrolled
Supervisor verification facts, not independent remote OS attestation. Neither
caller JSON nor an ordinary Observer report can supply a live content/launch proof.
The trusted installed verifier and OS, development release root and between-check
file-stability limitations remain explicit.

An owner proposes a multi-selection assignment using existing component IDs and
expected revisions, site parameters/dependencies and timing bounds. P allocates
stable run and instance IDs. S validates the actual plan, release and immutable
catalog before submitting a preparation. The owner approves that concrete,
current preparation. P records a finite shared-clock start grant and holds each
component against another active assignment. No automatic restart is authorized;
a new attempt requires an explicit new assignment.

S persists the assigned IDs as Prepared before OS effects. A private live grant
is obtained through the current execution session, compared to the actual local
preparation, and consumed through the existing RegisteredBackend. Its durable
registration assignment still precedes OS spawn. The backend must apply the
complete catalog requirements or refuse; Unknown is not NotRequired, and an
upper bound is not a reservation. The grant does not assert that resource
application or functional readiness has already succeeded, and is not work use.
Existing physical lifecycle/Host gates remain required for unsupported effects.

P-attributed local snapshots are separate from legacy S author rows. Entering
managed mode seals local declaration/index/freeze writes. The new service role
and operational records opt into reader schema 9; ordinary stores remain 6,
source-only seals 7 and intake-only stores 8. Promotions are monotonic. Older
readers must refuse operational stores rather than bypass active assignments.

Grant replies and source records retain the original assignment/instance identity.
A new P or S incarnation cannot replay an old grant as a fresh start. TTL is not
proof that a granted attempt never executed; unresolved grants retain their
claims. Source observations distinguish Running, Exited, NotStarted and Unknown,
and never use a PID alone as ownership. Stop of an owned software child remains
local during a P outage. Network delivery must not block the management loop.

The implementation must establish a real owner-to-S software execution and stop
path plus response-loss/restart/counterexample evidence. Active-unknown recovery,
complete logical/physical resource bundles, functional dependencies, automatic
packaging and the whole RF01–RF14 model remain required work; this contract is not
a completion claim. This initial source/runtime path is one shared Linux boot
clock domain; cross-host clock/delegation semantics are not inferred from it.

## Public workflow and explicit support limits

P configuration adds `resident_supervisors`, keyed by Supervisor principal. Each
enrollment pins one canonical registry-path digest, allowed verified release
digests and program name -> full catalog digest/effect mappings. The optional map
is empty by default. Ordinary user sessions refuse the Supervisor role, and the
separate mTLS execution service requires that role alone plus matching enrollment.
Its Open request binds the actual local registry and shared Linux clock context.
Imported nodes additionally retain their original frozen cut and registry binding;
a different empty registry cannot replace that source. Existing nonempty legacy
registries must first finish source/target reconciliation and resolve old execution
obligations. This is one shared filesystem/boot domain; namespace aliases and
whole-file rollback require separate explicit migration/restore work.

Owner HTTP endpoints use the existing authenticated browser/CSRF boundary:

- POST `/api/v1/resident-executions`: Propose, wrapped in the original request key.
- GET `/api/v1/resident-execution?id=UUID`: current authoritative assignment record.
- POST `/api/v1/resident-executions/approve`: expected assignment revision, exact
  preparation digest and a 100..30000 ms start window.
- POST `/api/v1/resident-executions/stop`: expected assignment revision; cancellation
  before grant or a recorded stop request after grant.

The optional mTLS service exposes Open, Inspect, Prepare and Observe. Prepare
uses actual source verification from the enrolled Supervisor. There is no endpoint
through which that service can approve its own assignment. Observer, Host and
ordinary author identities cannot act as the execution peer. Read-only delivery
of a stored grant is not a new grant: S returns at most one private live capability
per assignment/client incarnation, and the local registry consumes each instance
before OS invocation. Local owner identity prevents a different manager object
from rewriting an earlier execution's observations.

`rx-solutionsd catalog CONFIG` verifies the actual installed release, records its
release floor, and opens the configured registry to emit authoring/enrollment
data. It does not need an assignment or connection and does not start processes.
`rx-solutionsd platform-run CONFIG` obtains an assignment, submits concrete
preparation, waits for owner approval and uses the existing management loop.
A received grant fixes the complete locally built launch, including arguments and
file hashes, as well as plan/run/instance context. The source cannot replace it
with site JSON, an old persisted grant, or a transfer acknowledgement.

This path currently forbids declaration changes/retirement while a granted
assignment retains the component claim. Stop and reconcile it first; general
online replacement remains RF06 work. A normal terminal software report releases
that logical component claim. It is not physical resource release or business-work
completion. Unknown/residual outcomes retain claims. There is no automatic
active-unknown resolution or reassignment to a new peer in this revision.

A View's phase is based on the last accepted source observation; RUNNING can
remain after an unreported local stop. `peer_current` compares protocol identity
and enrollment, not heartbeat or current OS ownership. Content receipts describe
an enrolled verifier's checkpoint, not continuously immutable disk. Functional
readiness, physical completion and work-use permission are separate axes.

The delivery worker persists original requests before sending and retains their
keys on response loss. It can coalesce observations before creating a request,
but cannot replace an entered request with a later snapshot. Network delays do
not run in the 50 ms management loop. Local software stop remains possible; a
finite delivery tail can end with an unresolved outbox and retained P claims.
Restarting the same original assignment does not create another process or rewrite
its source history. These boundaries do not substitute for RF07/RF08 recovery.
