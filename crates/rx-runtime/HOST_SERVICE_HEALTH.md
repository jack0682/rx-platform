# Current-state diagnostics for Host execution services

2026-09-11. Connected the state of platform-managed Host connection, inspection, and delivery services to the operator view. This information is **diagnostic in-memory state** and is not input to qualification, Host grants, dispatch permits, or operation outcomes.

## Ownership and lifetime

A dedicated registry in `rx-runtime::Application` manages state inside the writer. It does not continually append heartbeats to SQLite/control ledgers. Recreating Application leaves the registry empty; it does not restore a healthy indicator from an earlier execution.

The local supervisor initially registers configured targets (host/cell). It validates the complete target set, duplicates, and the maximum of 64 before applying it, preventing partial configuration when a later entry is invalid. An uninitialized environment shows diagnostics unattached; initialization with no targets shows unconfigured; configured targets without reports show awaiting first report.

Each target receives an opaque owner ID bound to runtime boot and cell definition/envelope. Owner IDs are not exposed to the browser. Reports must match that owner and an increasing sequence. Old owners, duplicate/regressing sequences, different clocks, and future activity times are rejected. At display time, a mismatch with the current definition/envelope is shown as a configuration mismatch.

The internal `ReplaceHostService` command replaces **only the diagnostic reporting owner**. It does not restart actual processes or devices. Existing reports are discarded while awaiting the new owner's first report; late reports from the old worker cannot overwrite it. Actual restart/rebind authority requires a separate procedure.

## Three service states

| Item | Value source | Interpretation |
|---|---|---|
| Connection/lease coordination | ConnectionService Waiting/Connecting/Bound/Attention/Stopped | Bound means the connection registration path completed, not proof of a live TCP connection at every instant |
| Observation inspection | ObservationReader results and P receipt time | Receipt is distinct from value validity. Continuity problems, inspection unavailable, and waiting are displayed separately |
| Command delivery loop | P time obtained after Dispatcher completes one finite processing pass | Recent loop activity, not evidence of command success, native completion, or an empty queue |

If any delivery error has been recorded, its history is shown separately. The history itself is not converted into current operation failure. Raw error strings, delivery/reconciliation counts, and operation IDs from other cells are not sent to the UI. Dispatcher is a Host-scoped service, so its activity is not a completion count for a particular cell.

## Report time differs from activity time

The relay forwards the latest state every 500 ms by default and on connection-state changes. It does not enqueue a separate diagnostic message for every rapid observation change. When the queue is busy, the latest state is forwarded again later; missing reports are not filled with healthy values. The last actual receipt time is preserved even after inspection failure.

Reports have a currentness limit of 3 seconds. Stopped reporting becomes STALE and is not displayed as healthy. **A fresh relay heartbeat does not advance the last observation-receipt or delivery-pass time.** If reports are recent but worker activity is old, the view indicates that the latest observation response/loop activity needs checking. These 3 seconds are the current diagnostic display policy, not a physical protection or real-time performance requirement.

The registry attaches diagnostics only to cells filtered by current API overview access. Read and decoration run sequentially within the same writer. UI display lifetime also cannot exceed remaining valid report/activity time and follows existing conservative expiry measured from request start.

## Actual startup and shutdown

`rx-platformd` configures Host services and allocates owners before starting the API service. It connects each ConnectionService's actual watch channel to the relay. Relay construction verifies that the owner's host/cell matches the service target. Wiring another service's activity to the wrong owner is rejected.

If no producer exists yet, the view shows waiting for Host authentication. It does not invent success based on registered leases or last-known source values. It reads the connection service's dispatcher and observation reports directly without guessing complete adapter/controller internals.

Diagnostic relays shut down as a separate task set from the core authority services. Shutdown attempts a final Stopped report and joins them within at most 2 seconds. Diagnostic delivery failure does not resubmit native operations or change authorization. Authority withdrawal, dispatcher drain, and physical-stop confirmation follow existing paths.

## UI, compatibility, and validation

Added an optional `rx.host-service-health.v1` snapshot to `HostDiagnostic.runtime`. Existing diagnostic values/condition decisions are preserved; absent diagnostics show unattached. The 8 base/cell normative files, 4 optional wire bindings, and 79 SDK files are unchanged.

Validation covers owner replacement/late reports, rejection of sequence regression, report expiry, separation of heartbeat/activity times, current user scope/authority withdrawal, rejection of previous owners by a new Runtime, prevention of partial inventories, and authentication-wait reporting in actual P startup. Host TLS integration also checks actual dispatcher-pass times.

Browser service-state branches are checked by having HTTP tests provide P-generated snapshots at the response boundary. This is not counted as physical-device state validation. Existing conditions, request recovery, intervention ACK, and mobile flows are preserved.

Actual ROS/controller/GPU process health, site interruption response times, automatic Host process placement/restart, and a complete installation/restore supervisor have not yet been implemented and accepted. A complete service summary in the local status file and operator support-log inspection also remain future work. The first physical cell is NOT_COMMISSIONED.
