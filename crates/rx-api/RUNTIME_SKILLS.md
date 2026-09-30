# Installed process views for runtime clients

These additive authenticated HTTP reads expose existing P records to a skill-facing
client. They neither register arbitrary skills nor grant execution authority.
They do not change the Host wire SDK or normative cell-operation contracts.

- `GET /api/v1/runtime-skills`: `rx.runtime-skill-catalog.v1`, filtered by the
  current principal's cell scope. Each entry identifies the installed process,
  configuration revision, commissioning state and blocking records. At most 128
  entries are returned, with an explicit `truncated` flag.
- `GET /api/v1/runtime-skill-result?run=UUID`: `rx.runtime-skill-result.v1`,
  authorized against the Run's cell. It returns the original Run, parts and work
  operation outcomes/dispositions. It reads the immutable Run configuration;
  `current_binding_matches` distinguishes it from a later cell configuration.
  At most 256 parts and 256 work entries are returned. `details_truncated` means
  the result is incomplete. `result_owner` is `PLATFORM`.

Binding identity includes installation identity, store generation, cell and the
configuration artifact. The supported invocation mode is `BOUND_CONFIGURATION`.
A client still invokes ordinary CreateRun, reads the current start context, then
submits StartRun. Each mutation retains its original request key and body; start
admission remains subject to the existing authorization, qualification, revision,
resource and Host/Executor checks. Catalog reads never arm a Host.

StartAttempt.expected_run_revision identifies the Run revision after installing
the pending attempt, so it is the successor of StartRun.expected_run. Retrieving
the original StartRun request can return a later state of the same attempt;
a receipt is not a second execution and not proof of native success.

The cross-repository `tools/test_runtime_skills.py` checks the installed consumer
against actual P/Executor/Host FILE_SIMULATION, including consumer SIGKILL after
StartRun response and recovery without duplicate native effects. It uses test-only
commissioning material through public routes. It does not prove physical readiness,
P/Host restart reconciliation, dynamic skill I/O or a complete runtime installer.
