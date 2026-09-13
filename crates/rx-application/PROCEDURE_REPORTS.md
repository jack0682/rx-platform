# Procedure fact recording and state promotion

Storing an external procedure report is separated from promoting case state based on that report. RecordProcedure does not execute native reset, isolation or robot movement commands. The current implementation reaches PROCEDURE_ACTIVE and REVALIDATING; READY_FOR_RESTART/production restart are separate future work. Non-operating clearance/closure is connected through a [separate path](NON_OPERATING_CLOSURE.md).

## Policy trust boundary

A Procedure Policy includes cell/definition/envelope, external procedure documents and dependency evidence artifacts, allowed case types, entry conditions, per-step action/actor/conditions/required observation sources, and maximum report age. Actual bytes/hash/schema are checked and the content is preserved by content address.

Engineer authority alone does not approve a policy. QualificationAuthority's separate verify_procedure must verify it, and the default implementation always rejects it. Current authority and current configuration binding are rechecked at every state promotion. Policy admission tests in this phase use an explicit simulation authority. This does not represent completed verification of a physical release/package verifier or actual procedure documents.

## Report format and ledger

rx.procedure-assertions.v1 contains case/existing revision/procedure digest/actor/action/UTC/scope/source, source-event ID, step, human and physical targets and typed claims, evidence IDs/observation times/validity periods/reported/unknown. The earlier notification ACK format can be read through optional-field defaults. UTC is for recording; validity decisions use the current P clock.

The current source of human procedure reports is an authenticated Operator/RecoveryLead account. The actor/source must match the actual identity, and record metadata must agree with the artifact bytes/hash/size/schema. WorkStarted/WorkFinished report the actor's own work; another person's completion cannot substitute for it. Equipment conditions are checked against existing Host FactRecord source/session/age/quality and defined conditions. Free-text claims are not executed as machine conditions.

Record IDs, source events and content are preserved. Different content under the same ID leaves a conflict/block instead of overwriting the existing record. All reports are recorded in the existing control/audit ledger. A new physical fact performs required closure invalidation. Actual work reported in a diagnostic case changes it to FAULT_RECOVERY and a latched state. Configuration reports invalidate production qualification.

## Handling stale requests

After validating current authentication/access and source/ID/content, the actual report is recorded first. If the requested case/cell revision is stale, no newly permitted state is created and transition_error=STALE_REVISION is returned. Already stored facts and blocks are not rolled back. The same key/body returns the initial receipt, so it does not add a physical change or case record again.

A storage failure is not reported as a recorded fact. A lost post-commit response is retrieved using the original key. Failure to expand permission and acceptance of facts are distinguished by facts_recorded/record/transition_error in the Receipt.

## State decisions

- ACK only records notification acknowledgment.
- EntryConditionsReported requires the current responsible person's RecoveryLead role, admitted policy and actor/step, complete impact scope, current Host fence ack, and P's entry conditions, evidence and age. reported=true alone does not permit entry.
- WorkStarted/WorkFinished preserve external facts and record each person's progress. Expanding procedure state requires the current policy and continuity of the previous entry context. If another cause changes the epoch, the old entry record cannot be used.
- PersonnelAccounted/HandoverAccepted require the current lead to confirm the entire nonempty participant set. One person's completion or an empty list does not clear other people or unknown states.
- Once work completion, personnel accounting and handover for all known participants are collected, the state is REVALIDATING. This is not readiness for restart.
- Isolation/Reset/Configuration reports invalidate the existing entry and require fresh confirmation. configuration_changed also blocks promotion into a new entry and requires subsequent change/qualification procedures.

## API

The development HTTP `/api/v1/cases/procedure` accepts a typed Submission and inline assertions and executes the same P transaction. Promotion failures include HTTP 409/403/422 and similar statuses, facts_recorded, case_id, record_id and the complete receipt. The caller must preserve the fact that an error was returned after facts were stored.

The frozen Cell.RecordProcedure accepts an already stored case/actor-bound assertion artifact. Currently, adding Operator/RecoveryLead roles to a service account to submit human reports is rejected. Human gRPC reports need a future binding that connects a verified user and registered terminal to the OPERATOR_API call. Actual users' direct HTTPS/development HTTP uses the same application handler. State-promotion errors include rx-procedure-facts-recorded, rx-procedure-record-id and rx-procedure-case-id metadata. The inline-bytes path for the first human report is currently development HTTP.

CaseDetail distinguishes notification ACKs, procedure records and per-person procedure progress. ACK counts in the operational list are not mixed with physical report counts. Actual procedure input/review screens are future work.

## Verification scope and limitations

The core checks fact preservation across stale-CAS/rollback/lost commit responses, entry rejection before actual fence/condition evidence, independent completion by two workers and full personnel accounting/handover, and rejection of old entry after an external epoch change. HTTP checks that a stale report preserves its record ID and exactly one fact/block even when the response is an error. Existing ACK/UI, execution/handover and service paths are regression-tested.

RX has not safety-certified actual work and equipment state through observation alone. This is a software boundary for recording/checking sources and evidence from defined external procedures and protective functions. Actual policy admission, physical procedures and product acceptance have not been performed. Recovery plans/guarded operations, restart and closure of scopes with no entry/unknown status, qualification of new configurations, and complete operator service identity/screens/artifact catalog/ledger retention remain outstanding.

Physical-change reports invalidate previous personnel-accounting and handover records. Retransmitting an already stored record under a different request key does not reapply previous work transitions. Preservation of new facts after closure follows the [non-operating closure specification](NON_OPERATING_CLOSURE.md).
