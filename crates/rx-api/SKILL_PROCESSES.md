# Skill process draft and versioned metrics

This unreleased continuation extends the LOCAL_SIM service with server-owned
skill composition. It does not complete the broader RX framework draft and does
not connect software results to physical P/Host/Executor authority.

## Definition and ownership

`POST /v1/processes` registers an immutable version containing the existing
`rx.process-source.v1` source, process input fields, skill bindings and exported
outputs. The shared `rx-process-contract::source_validation` validator checks
structure before this profile accepts the definition. Only one condition-free
flow with Sequence/Operation nodes is currently supported. Other node kinds are
rejected, never silently flattened.

Each operation binding refers to a registered skill name/version. Registration
pins its exact package digest. Every input port is bound to a process INPUT field,
a preceding step OUTPUT field, or a LITERAL. Missing, forward and incompatible
references are rejected. Binding names must be distinct so references cannot
silently choose one of multiple occurrences. A process contains 1–64 calls;
registration and admission are bounded to 128 process versions/runs.

`POST /v1/process-runs` accepts a caller-retained request UUID and business inputs.
The existing state writer atomically stores the parent record and reserves every
child UUID. A standalone submission cannot take over a reserved child UUID.
Each child is admitted only after its predecessors have valid successful results;
its input is resolved from this parent and those exact executions. Neither the
CLI nor the worker decides the next business step. The worker continues to execute
one ordinary skill at a time through its existing API.

The original parent and child IDs survive response loss and process restart.
Changing input under the same parent key is rejected. UNKNOWN, disputed or failed
predecessors block later calls. Source, input and skill version remain frozen.
Insufficient or oversized composed input records a BLOCKED issue without
poisoning unrelated work in the worker queue. Parent views expose NOT_STARTED
steps as absent executions; they do not manufacture NOT_EXECUTED receipts.

This profile owns no physical resource set, so a successful software result is
not a handover/RELEASED assertion. Device workflows still need the existing
platform eligibility, Host facts, condition freshness and resource disposition.
Operator reconciliation, branch/parallel execution and physical integration
remain incomplete requirements of the larger goal.

## Read surfaces

- `GET /v1/processes`: immutable registered definitions and package pins.
- `GET /v1/process-runs`: parent views with ordered skill calls.
- `GET /v1/process-runs/{id}`: original parent, reserved child IDs, input/output
  lineage, state and issue. GET never advances work.
- `GET /v1/metrics?since_ms=...&until_ms=...`: a projection from one repository
  snapshot, identified by the actual durable journal sequence.

Metric windows select child/standalone skill admission times, inclusive since and
exclusive until. Groups are `(skill, version, package_digest)`. Successful, failed,
unknown, canceled, not-executed, active and queued states stay separate. The
reported success fraction explicitly uses only `(succeeded + failed)` as its
denominator and is null when that denominator is empty. It is not a first-attempt
or good-part yield measure: user-created retry requests are distinct executions.

Execution duration statistics use worker-reported monotonic elapsed measurements
for known successful/failed computations. Queue wait is a separate server-wall-
clock difference; backwards timestamps are excluded and counted as missing.
Statistics expose sample/missing counts and nearest-rank p50/p95, not zeros for
absent data. They are not robot performance or physical deadline guarantees.

## Compatibility and validation

The optional `Run.parent` is absent for preexisting standalone records. New readers
retain those records. Older binaries cannot read process-child records, so an
automatic downgrade is not supported. Common cell wire contracts and exported
Host SDK bytes are unchanged.

Focused tests exercise preserved child identities across restart between steps,
same-parent replay, future child takeover refusal, UNKNOWN blocking, forward/type
reference rejection, composed-input size isolation and version/window metric
denominators. Installed acceptance runs four authored Python skills with two
different material IDs, actual exceptions and timeouts, and observes one worker
execution per child. This is logical software data-flow evidence only.
