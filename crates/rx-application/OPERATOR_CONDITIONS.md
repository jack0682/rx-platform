# Condition and observation diagnostics in the operator UI

2026-09-11. Shows operators which conditions and evidence to check when preparing execution. `diagnostics` was added to each cell in `GET /api/v1/overview`; this read creates no operating permission/qualification/Arm/native commands.

## One read and current access

Read cell configuration, Host registration, source observations and condition expressions in the same transaction as the existing overview. Filter cells first by the current account/terminal access scope, excluding other cells' observations/conditions from the response. Recheck access on every read. `snapshot_id` identifies this read; it is not a control-journal cursor.

The new model is `rx.cell-diagnostics.v1`. It does not manufacture frozen `ConditionEvaluation` condition IDs/revisions or clearances. Paths such as `Start//1` and `Operation/step/0/1` are display paths bound to current configuration and cannot be reused as official condition-contract IDs. Existing base/cell normative files, four optional bindings and 79 SDK files are preserved.

## Distinct displayed states

| Item | Evidence and meaning |
|---|---|
| Operating qualification/blocks/interventions | Existing CellSummary. Missing qualification/existing blocks remain visible even when conditions are satisfied |
| Device integration authority | Check stored Host registration, current authentication, epoch/scope and grant clock/expiry. Not evidence of a TCP connection or actual device readiness |
| Pre-start checks | Evaluate current configuration's start_conditions with the domain evaluator |
| Conditions maintained during operation | Evaluate maintained_conditions with the same evaluator |
| Per-operation entry conditions | Each StepBinding's conditions. Not advance approval of the complete work plan/budget/resource acquisition |
| Observation evidence | Last value/acquisition time/age at read time/maximum age/quality/uncertainty/registered and observed generations/evidence ID |

A usable false observation is distinct from an unusable observation. If false is a condition's expected value, that condition can PASS. Observation validity alone does not imply success/failure or device readiness.

Source usability and every condition verdict use existing domain `Condition::evaluate`. Diagnostic reasons explain absent values, generation/provenance/format/unit mismatches and quality/clock/uncertainty/age problems. UNKNOWN caused by a usable source not matching a condition expression's expected type is explained separately.

Fact construction for condition evaluation is shared too. Maximum age at storage time does not override current policy; quality is treated as unusable if source Host/acquisition uncertainty does not match current FactSpec. These rules apply to diagnostics and actual admission evaluation alike. Records themselves are not modified.

## Display validity

The server also returns `display_valid_for_ns` relative to read time. The current cap is 3 seconds, shortened to the remaining lifetime of usable source evidence or a valid Host grant if smaller. This is a conservative limit for retaining a displayed judgment, not an operating permit TTL or physical response time.

The browser uses **monotonic request-start time + the server's remaining lifetime**, rather than starting a new timer upon response receipt. Network/response waits therefore do not extend display validity. Once the limit passes or a read fails, condition/observation/Host-authority badges change to 'Refresh required', preserving the earlier judgment separately.

Observation age/value are labeled 'at read time' and 'last observed value'. The browser does not fabricate a new native observation or current device time. Display refresh depends on browser scheduling and is not used as real-time protection. Every actual write must pass existing server checks again.

## UI and actions

Added an 'Operating conditions' menu in the existing workspace and a navigation button on the operator screen. This page supports reads and expanding evidence. Existing 'Prepare new run' remains record preparation; the page's PASS does not extend it into a start action. Existing intervention ACK/same-request recovery/preservation of reviewed revisions are also validated unchanged.

Human-facing source names are not registered yet, so configuration IDs for source/Host/step are displayed. Meanings such as 'Door closed' or 'Chuck locked' are not invented. Future integration will use names/descriptions from device/process packages.

## Validation scope

- Application tests verify usable false versus UNKNOWN, observation expiry, source-generation changes, condition-expression type mismatch and no authority elevation by reads. Shared application of current source identity/uncertainty policy is also tested.
- HTTP tests verify no exposure of other cells, UNKNOWN for unregistered observations and read rejection after current-account revocation.
- The browser's no-observation state is validated with an actual local P service response. PASS/FAIL/expiry display is checked by supplying actual read-model JSON generated by application tests at the browser response boundary. This does not inject device observations or qualification into the running service.
- Response delay must not extend display validity. Desktop/mobile, JavaScript errors, existing login, lost request responses/recovery, intervention ACK and configuration reads are checked together.

Field acceptance connecting actual ROS/PLC signals to this screen, integration of actual ROS/controller and other subordinate-process status, complete condition-contract wire mapping, description translation/device display names, paging/SSE for large reads and recovery-action UI remain outstanding. The first physical cell is NOT_COMMISSIONED.

Current diagnostics for platform-managed connections/readers/dispatchers are connected through [Host service health](../rx-runtime/HOST_SERVICE_HEALTH.md). Registered Host authority context and service activity are displayed separately.
