# Admission, holding and notification acknowledgment for intervention cases

Status: case creation/reads and notification acknowledgment are connected to P transactions, executor mTLS and development HTTP/operator UI. A subsequent stage connected [policy-based procedure reports, per-person end/handover and REVALIDATING](PROCEDURE_REPORTS.md). Recovery plans/clearance/restart remain incomplete. An acknowledgment button cannot bypass these states.

## Opening a case

Check current Operator/RecoveryLead/Executor/Host permissions and access to the requested cell, then compare the complete key/body. The lead must be an actual active RecoveryLead account with access to impacted cells. Supplied operation IDs must be actual records in impacted cells. Material IDs are reported references; this API does not create MaterialState with verified material identity/location/support.

Since the current authority unit is the cell, impact extends through the cell closure connected by the current cell's scopes/shared resources. Unknown scope/material references are preserved as scope_uncertain. A requested narrow scope is not directly treated as a safety boundary.

| Case type | Atomic handling | Initial state |
|---|---|---|
| DIAGNOSTIC_ONLY | Add non-latched blocks and case membership to impacted cells. Preserve epoch/existing ACTIVE mandate | OPEN |
| PLANNED_ACCESS / FAULT_RECOVERY / MAINTENANCE / CHANGE_REVIEW | Existing invalidation path revokes epoch/fence/mandates, seals unentered permits and records case membership | CONTAINMENT_PENDING |

Diagnostic cases also hold related new admissions. Correlated results of existing invocations can still be recorded. If actual physical access/change/loss of continuity is reported, the case cannot end as diagnostic-only; that report/transition path is follow-up implementation. Diagnostic cases are not automatically cleared currently.

The new case, block/authority handling, event and response share one commit. The same key/body after a lost response recovers the same case and initial response. Other cases' blocks/membership are not removed. Shared impacted cells can read the same case.

## Notification acknowledgment

Case.Acknowledge records only the current account's notification acknowledgment. It stores the actual actor, recorded time, reported RFC3339 UTC time, case revision/scope at that point and a content-addressed rx.procedure-assertions.v1 artifact. The assertion explicitly contains authenticated-notification-ack and empty physical_claims. UTC is for recording, not calculating command validity or access conditions.

The case record ID list and revision increment, but case state, participants, cell epoch, blocks, mandates, operation outcomes and access/reset/restart permissions do not change. Replaying the same request adds no acknowledgment record. Different bodies for the same key, stale case revisions and unauthorized accounts are rejected. This ACK path is not presented as general ProcedureRecord handling.

## Transport and UI

- Frozen Cell.OpenCase/GetCase are connected to registered executor identity. Because OpenCase adds restrictions, a valid historical expected_cell_revision is not rejected as CAS, but remains part of the key's complete intent.
- Development HTTP routes are `/api/v1/cases`, `/api/v1/case`, `/api/v1/cases/open` and `/api/v1/cases/acknowledge`. They use existing development cookie/same-origin/body rules and do not claim completion of the entire frozen operator HTTP projection.
- The operator UI shows impacted case lists, state, lead, restrictions and acknowledgment records. ACK uses the existing browser pending-key flow. Procedure catalogs/case creation UI, physical procedure input and restart UI are follow-up work.
- RecordProcedure was connected in a subsequent stage to typed assertions/current actor/policy checks. Recovery/clearance RPCs remain incomplete; ACK is not a substitute for those actions.

## Validation

Core tests verify creation transaction rollback/lost responses/same-case recovery, latched versus diagnostic holds, preservation of UNKNOWN results, no authority elevation by ACK, preservation of other cases' restrictions and duplicate keys. HTTP tests check rejection of read-only accounts and replay of the same ACK; mTLS tests check executor Open/Get and rejection of unsupported procedures.

Browser tests prepare a case through the actual development API and lose its ACK response after commit. After page reload, the same body/key recovers the response; tests check one record, CONTAINMENT_PENDING and unchanged epoch/block. Desktop/mobile display and JavaScript errors are also checked. Procedure references and cells are synthetic data; no actual device or physical access tests are performed.

Follow-up work includes typed external procedure evidence/fact preservation even with stale CAS, procedure/participant/handover policies, recovery plans and unique step slots, clearance cohorts, explicit restart/non-operating closure, journal paging and production deployment.
