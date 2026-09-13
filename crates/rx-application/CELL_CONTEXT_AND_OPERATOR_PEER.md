# Recorded cell state and operator API service sessions

2026-09-11. Cell mode, commissioning state and block creation revision are persisted and connected to frozen `Cell.Inspect` and human-facing HTTP reads. An operator API service can also negotiate as `OPERATOR_API`. Authentication of this service does not substitute for a person's start or recovery intent.

Normative sources are [CellContext and roles](../../spec/cell_operations/v1.0/04_protocol_integration_ui.md) and [base Session/PeerHello](../../spec/contracts/v1.0/03_data_and_protocol.md). The eight normative files and manifests are unchanged.

## Information stored for a cell

| Information | Representation | Interpretation |
|---|---|---|
| mode | SETUP / AUTOMATIC / RECOVERY / MAINTENANCE | The operating context RX is handling. The actual robot/PLC mode selector value is a separate observation/condition |
| commissioning | NOT_COMMISSIONED / COMMISSIONED / REVALIDATION_REQUIRED | The currently recorded qualification for use and need for review. Distinct from an operating-condition PASS or a physical safety judgment |
| block.created_revision | The actual cell revision at which the block was first persisted | Not overwritten with the revision at read time |
| block.case_id | ID of the case that created the block | Not mixed with another case's blocks or historical closure records |

The new metadata is optional in the internal stored models for `Cell` and `Block`. Missing values in older records remain None. New cells and blocks receive values at the actual write boundary.

## State transition boundaries

| Event | mode | commissioning |
|---|---|---|
| New cell registration | SETUP | NOT_COMMISSIONED |
| Qualification registration with exact configuration and evidence | Unchanged | COMMISSIONED |
| First production start commit after all Host Arm confirmations | AUTOMATIC | Recheck current qualification |
| Setup start commit after all Host Arm confirmations | SETUP | Recheck current qualification |
| latched invalidation | A simple HOLD/reboot preserves the existing context | Existing COMMISSIONED becomes REVALIDATION_REQUIRED |
| Non-diagnostic intervention case creation | MAINTENANCE for maintenance cases, otherwise RECOVERY | Existing qualification requires review |
| Report of an actual procedure change | MAINTENANCE/RECOVERY appropriate to the case | Apply existing invalidation rules |
| Non-operating closure commit | MAINTENANCE | Preserve the OUT_OF_SERVICE block and review-required state |

Diagnostic cases and notification ACKs do not change operating mode or qualification. AUTOMATIC is not displayed merely because a start was requested. Cell state changes together with all Arm confirmations and the Run/Mandate commit. The pre-start cell revision therefore cannot be reused for CAS on later operations.

Runs with different purposes are not activated concurrently in contradiction to a cell's scalar operating context. A setup start is rejected when a production run is already executing. This does not constitute a complete implementation of general parallel processes or resource scheduling.

This mode field does not operate native ModeGoal or a physical selector. Actual device mode, entry conditions and conditions for continued operation require separate profile/condition evidence. The UI labels it 'RX operating mode'.

## Blocks and revisions

A block is bound to the revision of its first update to that Cell. For example, if invalidation creates a block at revision 8 and the same transaction stores case membership at revision 9, `created_revision=8`. Later ACKs, reads or other changes do not modify this value.

Intervention/procedure handlers attach case_id only to blocks they created. They do not assign arbitrary case IDs to shared HOLD/RuntimeRestart/OUT_OF_SERVICE blocks. The rule that non-operating closure does not clear another case's blocks remains in place.

## Public reads

| Path | Authentication and scope | Result |
|---|---|---|
| gRPC Cell.Inspect | Registered mTLS certificate, current base session, exact cell manifest/definition negotiation, current cell access | frozen CellContext |
| GET `/api/cell/v1/cells/{cell_id}/inspect` | Existing HTTP user session, current role/cell scope checks | RX JSON of the same CellContext |
| Existing `/api/v1/overview` and operator UI | Current per-user projection | Additional display of stored mode/commissioning |

Percent-encode the path segment when cell_id contains `/`. Example: `cell/a` → `cell%2Fa`.

gRPC Inspect does not consume a mutation key or expected revision. Specifying either is rejected. Cell, access rights and negotiated binding are checked in the same writer read transaction; mode or block information is not fetched again in a separate transaction and mixed in.

The converter rejects absent metadata, 0/future created_revision, duplicate block/open_case IDs, scope vector mismatches, and inconsistency between COMMISSIONED and the qualification reference. Older records without metadata produce UPGRADE_REQUIRED. Other storage-integrity errors are distinguished as DATA_LOSS/STORE_FAULT. Numbers, enums and optionals use frozen RX JSON.

This CellContext read does not provide a filtered control-journal cursor or SSE. Complete CellJournal wire projection/paging/subscription remains separate follow-up work.

## Operator API service identity

`Role::OperatorApi` is a transport service role. It is not treated as the same enum value as a regular Operator/RecoveryLead. Registered accounts have only OperatorApi and optionally Observer roles. Operator API negotiation is rejected for accounts that mix in other human/Host/Executor privileges. This service account cannot open a regular user login session either.

- Session.Open supports `OPERATOR_API`. An evidence journal/last_seq is not permitted.
- Authentication binding covers the certificate fingerprint, installation, store generation and release digest.
- Reconnection with the same boot/binding/current session retains the same session. A new boot/binding retires the previous service session and clears cell negotiation state. Re-entry by a retired boot is rejected.
- Cell.Open negotiates exactly one currently accessible definition. Ambiguous definitions or another cell are not permitted.
- Cell.Inspect rechecks the current account, role and scope, and session/certificate/definition on every call.
- Connecting or reconnecting this service does not change the cell epoch/Run/Mandate or executor session.
- Service authentication alone does not grant Cell.OpenCase, operation/recovery/start, or Executor GetRun/Submit permissions.

Actual human action input currently reaches the application through direct terminal HTTPS and development HTTP using an authenticated user Identity. Delegation/session binding that associates a person with an operator API service's gRPC writes is not yet implemented. The service is not promoted to RecoveryLead or Executor to fill this gap.

The subsequent integration must distinguish evidence for the current user session, service session and registered terminal. It must not accept actor/session strings in the body as identity evidence, and must check the person's current authority on every mutation before consulting the existing idempotency cache. Frozen human-write RPCs such as PrepareClose/CloseWithoutRestart are enabled after this boundary is connected.

## Storage and rollback boundaries

This stage introduced the schema4 barrier. The current user/terminal binding implementation extends it to [schema5](TERMINAL_IDENTITY.md). Older runtimes that do not know the new fields reject this store through their `version > 3` check. Replacing only the binary must not allow an old decoder to read or partially update new Cell documents.

Migration 0004 raises only the compatibility barrier without changing existing entity/request/event/outbox/control data. It does not infer missing mode/commissioning/block creation points. Reviewing and reconnecting older records, and explicitly filling their metadata, are follow-up procedures. Returning to an older binary and restoring an older data snapshot are different operations; the complete P/H restoration and native-result reconciliation specifications cannot be skipped.

Tests create actual schema 3 tables and document bytes and verify unchanged bytes after schema 4 opening and backup. Opening a version newer than the current decoder is also rejected. The `version > 3` condition in the archived historical code was checked separately. This does not mean that actual deployment update or field restoration testing is complete.

## Validation and remaining scope

Application tests verify start/intervention mode and qualification transitions, actual block creation revision, conflicts between run purposes, and operator API reconnection/mixed roles/preservation of executor authority. HTTP tests verify current user scope, frozen JSON and rejection of missing legacy metadata/future revisions. Actual TLS fixtures verify reads before/after OPERATOR_API negotiation, scope denial, denial of write/executor permissions, and new/retired boots.

The operator UI distinguishes the presence of a qualification record from the need for revalidation and displays RX operating mode. Browser checks covered actual login, unregistered state, mode display, recovery after a lost response, and mobile scope. Visual acceptance for a qualified physical cell has not been performed.

Key remaining work includes the boundary linking human sessions/terminals to service calls, frozen human-write RPCs, explicit mode changes/qualification updates/legacy metadata completion, recovery/restart, the complete journal and product deployment. This implementation does not establish validation of actual robot/PLC modes or safety functions.
