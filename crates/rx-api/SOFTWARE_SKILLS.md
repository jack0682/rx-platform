# Local simulation skill service

The unreleased [skill process draft](SKILL_PROCESSES.md) adds server-owned
composition, child execution lineage and versioned metric projections. It is a
continuation toward the full framework goal, not proof of device integration.

The `rx-skill-server` executable is a developer preview for registering versioned
Python computations and recording their execution. It uses `rx-application` policy,
the existing bounded `rx-runtime::writer`, `rx-storage::SqliteRepository`, and
`rx-domain::Operation`. Native Python execution belongs to the solutions worker.
No HTTP handler runs code or owns a separate result database.

This profile has its own installation and must not open an existing cell database.
It exposes no Cell, Host, qualification or physical-operation endpoints. It is not
a replacement for the existing P/H/E physical execution path. The package format
accepts only LOCAL_SIM. The separate application profile earns its place because
software computations do not establish robot qualification or physical handover.
Connecting the skill facade to qualified device capabilities is future work.

One skill is an immutable name/version, Python source, input/output field map and
deadline. Supported field types are string, integer, number, boolean, object and
array; nested object schemas are not interpreted. Every declared field is required
and extra top-level fields are rejected. Sources are limited to 256 KiB, inputs and
outputs to 64 KiB, deadlines to 100–60000 ms, packages to 128 and runs to 1000 per
installation. Limits reject new admissions without deleting history.

Requests carry a caller-retained UUID. The same UUID/content returns the same
Operation; changed content is rejected. The writer commits EXECUTION_ENTERED before
a worker receives the source. A lost claim reply is never permission to launch it
again. A restarted server or lost worker marks admitted execution UNKNOWN/UNRESOLVED
and does not replay it. Queued work has not crossed the execution boundary and can
be claimed later. Repeated identical result receipts are idempotent; conflicting
receipts and late receipts after quarantine are refused. No automatic retries.

The serial local worker is the authenticated source of computation results.
Successful output must satisfy the registered schema. Process exit is software
evidence only, and the service never claims physical resource release. Diagnostic
time stamps use wall time; execution duration is measured by the worker's monotonic
clock. Neither is a physical safety deadline or a qualified performance KPI.

The installer publishes the service on host loopback, uses separate random client
and worker tokens, isolated Docker networking, no device mounts and no host Docker
socket in containers. Only trusted authors should register code: the Python
runtime is resource-limited but is not a hostile-code sandbox. The worker and
server have separate containers and data volumes. This is a single-user local
development deployment, not an Internet-facing or multitenant service.

API: client token authorizes `/v1/skills` GET/POST, `/v1/runs` GET/POST and
`/v1/runs/{id}` GET. A distinct worker token authorizes internal claim, finish and
continuity-loss reporting. `/health` and the empty dashboard require no token;
records require authentication. The dashboard keeps its token only in page memory.

Validation includes immutable registration, changed-key refusal, output validation,
worker ownership, reply replay and restart quarantine. Release acceptance also
exercises the installed image, external authored skills, process failures, response
loss, persistent restart and refusal of physical manifests.
