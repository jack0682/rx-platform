# Data, APIs, Base-Contract Integration, and Operational UI

Normative: RX Cell Operations Contract v1.0. base=`rx.contract.v1`, extension=`rx.cell.v1`. In tables, `name:type#n` denotes field number, `?` optional, and `[]` repeated. Numbers are normative for this design, which was specified before generated code/builds existed.

## 1. Common representations and negotiation

Id/Name/Digest/TimePoint/CallContext/ArtifactRef/Observation/Evidence/Receipt and basic RPC errors follow base rules. Retain UTF-8, proto3 optional, rejection of unknown enums/fields, duplicate fields, NaN, and incorrect units, and JCS semantic projection. Enum 0 is UNSPECIFIED, not a valid business value. Number entries from 1 in the order listed below.

CellHello is `base_manifest_hash:Digest#1, cell_manifest_hash:Digest#2, peer_id:Name#3, base_session_id:Id#4, cell_definition_digest:Digest#5, shared_clock_id:string#6`. Peers must explicitly support both contracts and CellDefinition. cell_manifest_hash is SHA-256 of this folder's protocol_manifest.json JCS bytes, containing normative-document 01–04 hashes and the base-manifest hash. CellSession.session_id identifies the negotiation record; CellCall/context and RunMandate.executor_session_id use **base_session_id**. Negotiation records bind to the same authenticated peer/base session and become invalid when the base session ends.

Native writes in cells with this feature enabled pass **only through cell-aware Host gates**. Keeping base Host.AuthorizeDispatch on the network as a downgrade path that executes without a permit is prohibited. Base implementation functions may be invoked internally, but callers must not bypass mandatory cell validation. A simple `optional_permit` configuration is prohibited.

Existing maximum message/batch, snapshot, consumer-buffer, and retention limits follow base. Envelopes must specify permit TTL, clearance TTL, maximum preparation/Arm waiting time, and each condition's maximum age/acquisition error with evidence. Permit/clearance validity cannot exceed the relevant evidence's validity limits; Host grant expiry is checked separately. Preparation timeout indicates failure/reconciliation, not completed native stopping. Do not invent default safety values.

## 2. Core types

| Type | Fields |
|---|---|
| ScopeEpoch | `scope_id:Name#1, epoch:uint64#2`. Sort by ascending scope_id; reject duplicates/0/regression |
| CellRef | `cell_id:Name#1, cell_epoch:uint64#2, scopes:ScopeEpoch[]#3` |
| CellContext | `cell_id:Name#1, revision:uint64#2, cell_epoch:uint64#3, scopes:ScopeEpoch[]#4, definition:ArtifactRef#5, envelope:ArtifactRef#6, qualification_id:Id?#7, mode:OperatingMode#8, blocks:Block[]#9, open_case_ids:Id[]#10, commissioning:Commissioning#11` |
| Qualification | `qualification_id:Id#1, revision:uint64#2, envelope:ArtifactRef#3, state:QualificationState#4, evidence:ArtifactRef[]#5, dependency_hashes:Digest[]#6, reviewed_by:Name[]#7, limitations:ArtifactRef#8` |
| ConditionEvaluation | `condition_id:Name#1, condition_revision:uint64#2, verdict:Verdict#3, reason:CellReason#4, evidence_ids:Id[]#5, evaluated_at:TimePoint#6, valid_until:TimePoint#7, cell:CellRef#8` |
| Block | `block_id:Id#1, kind:BlockKind#2, reason:CellReason#3, scope_ids:Name[]#4, condition_id:Name?#5, case_id:Id?#6, created_revision:uint64#7` |
| RunMandate | `mandate_id:Id#1, run_id:Id#2, cell:CellRef#3, envelope_digest:Digest#4, actor:Name#5, origin:StartOrigin#6, purpose:Purpose#7, state:MandateState#8, budget:RunBudget#9, executor_session_id:Id#10, start_attempt_id:Id#11, restart_plan:ArtifactRef?#12` |
| RunBudget | `unit:BudgetUnit#1, limit:uint64#2, consumed:uint64#3, revision:uint64#4`. limit>0, consumed≤limit |
| PartAttempt | `part_attempt_id:Id#1, run_id:Id#2, ordinal:uint64#3, material_id:Id?#4, revision:uint64#5, disposition:PartDisposition#6`. MaterialState distinguishes identity evidence from physical confirmation |
| DispatchPermit | `permit_id:Id#1, operation_id:Id#2, intent_digest:Digest#3, cell:CellRef#4, envelope_digest:Digest#5, qualification_id:Id#6, qualification_revision:uint64#7, purpose:Purpose#8, parent:PermitParent#9, grant:base.Grant#10, host_boot_id:Id#11, issued_at:TimePoint#12, expires_at:TimePoint#13, conditions:ConditionEvaluation[]#14, state:PermitState#15` |
| PermitParent | oneof `mandate_id:Id#1` or `recovery:RecoveryStepRef#2`. RecoveryStepRef=`case_id:Id#1,plan:ArtifactRef#2,step_id:Name#3,visit:uint64#4` |
| StartAttempt | `attempt_id:Id#1, run_id:Id#2, status:StartStatus#3, target:CellRef#4, expected_cell_revision:uint64#5, case_revisions:CaseRevision[]#6, clearance_ids:Id[]#7, mandate_id:Id?#8, reason:CellReason#9`. CaseRevision=`case_id:Id#1,revision:uint64#2` |
| InterventionCase | `case_id:Id#1, revision:uint64#2, cell:CellRef#3, type:CaseType#4, state:CaseState#5, procedure:ArtifactRef#6, lead:Name#7, participants:Name[]#8, operation_ids:Id[]#9, material_ids:Id[]#10, record_ids:Id[]#11, block_ids:Id[]#12` |
| ProcedureRecord | `record_id:Id#1, case_id:Id#2, case_revision:uint64#3, action:ProcedureAction#4, actor:Name#5, scope_ids:Name[]#6, occurred_at:base.UtcTime#7, evidence_ids:Id[]#8, assertions:ArtifactRef#9` |
| Clearance | `clearance_id:Id#1, case_revisions:CaseRevision[]#2, cell:CellRef#3, evidence_ids:Id[]#4, prepared_at:TimePoint#5, valid_until:TimePoint#6, disposition:ClearanceDisposition#7, consumed_by:Id?#8, target_run_id:Id?#9, envelope_digest:Digest#10, restart_plan:ArtifactRef?#11`. RESTART requires target_run_id and restart_plan |
| MaterialState | `material_id:Id#1, revision:uint64#2, identity_basis:IdentityBasis#3, identity_evidence:Id[]#4, location_claim:Name?#5, supports:SupportClaim[]#6, scope:CellRef#7`. SupportClaim=`holder:Name#1,verdict:Verdict#2,evidence_ids:Id[]#3,valid_until:TimePoint#4` |
| ChangeRecord | `change_id:Id#1, revision:uint64#2, cell:CellRef#3, before:ArtifactRef[]#4, after:ArtifactRef[]#5, state:ChangeState#6, affected_ids:Name[]#7, evidence_ids:Id[]#8, applied_by:Name?#9` |

OperatingMode=SETUP,AUTOMATIC,RECOVERY,MAINTENANCE. Commissioning=NOT_COMMISSIONED,COMMISSIONED,REVALIDATION_REQUIRED. QualificationState=DRAFT,IN_REVIEW,QUALIFIED,SUSPENDED,RETIRED. Verdict=PASS,FAIL,UNKNOWN. BlockKind=TRANSIENT,LATCHED. StartOrigin=OPERATOR,BATCH_SCHEDULER. Purpose=PRODUCTION,SETUP,RECOVERY. MandateState=ACTIVE,REVOKED,EXHAUSTED,CLOSED. BudgetUnit=PART_ATTEMPT,OPERATION_COUNT. PartDisposition=IN_PROGRESS,CONFIRMED_COMPLETED,REJECTED,UNRESOLVED,NOT_PROCESSED. PermitState=ISSUED,CONSUMED,VOIDED,EXPIRED. StartStatus=PENDING,ARMING,STARTED,REJECTED.

CaseType/CaseState/ProcedureAction/ChangeState follow the enumeration order in 03. ClearanceDisposition=RESTART,REMAIN_OUT_OF_SERVICE. IdentityBasis=VERIFIED_SENSOR,TRACKED_SLOT,HUMAN_DECLARED,UNKNOWN_IDENTITY. All counters are decimal strings under base JSON representation. Human names are display metadata separate from immutable actor identity.

MaterialState is a multi-support claim, not a machine safety certificate. PartAttempt is a **run-owned** permitted-attempt budget unit. If the physical object changes, do not conceal it under the same part_attempt_id; link material identity/disposition/change records. PRODUCTION SubmitOperation requires a part_attempt_id from the same run, shared by multiple nodes. On restart, check whether the current mandate's restart_plan allows that continuation of the existing part. SETUP omits part_attempt_id and uses OPERATION_COUNT. RECOVERY requires RecoveryStepRef instead of a mandate and binds to the persistent unique visit slot in 03.

## 3. Required artifact-schema contents

The following schema IDs identify content formats. Do not use an artifact for business decisions without understanding its schema even if digest verification succeeds. Deploy content-addressed artifacts at identical revisions in both images; arbitrary URL fetching during execution is prohibited.

| Schema ID | Required semantic fields |
|---|---|
| rx.cell-definition.v1 | Cell ID, device/Host/native-endpoint membership, control and material/support resources, scopes/zones, dependency edges, external protective functions and source roles, conflict/independence evidence |
| rx.operating-envelope.v1 | Complete operating limits from 01 §3; profile/recipe/tool/fixture/calibration/material/environment hashes; purpose/mode/budget/source/role; condition/case/recovery permissions; timing and evidence; hazard/function/test references |
| rx.condition-definition.v1 | Condition ID/revision, applicable purpose/step and ADMISSION/MAINTAINED/WAIT_TARGET, typed grammar tree, sources/units/quality/age/conflict rules, affected scopes, TRANSIENT/LATCHED and local-reaction references |
| rx.risk-function-map.v1 | Hazard ID, work/mode/exposure/event/harm, operating scope, measures/function IDs, responsibilities, required-performance sources, validation, residual risks, unknown items |
| rx.safety-function-spec.v1 | Trigger, complete path allocation, state/maintenance, timing, performance/evidence, faults, reset/start, validation, constraints from 01 §7 |
| rx.recovery-plan.v1 | Case/procedure/envelope scope; finite step kinds, edges, visit limits, alternative guards, native continuation, failure/stopping, evidence, actors from 03 §4 |
| rx.restart-plan.v1 | Target run/recipe/envelope, original run/operation and current material/part_attempt bindings, new activation or CONTINUE_CORRELATED point/evidence, native-resume semantics, budget ledger, required restart conditions |
| rx.procedure-assertions.v1 | Procedure step and actor; actual external functions/actions, scopes/personnel/physical targets; sources/authority/validity; assertions/unknowns/evidence references |

Condition grammar permits only finite trees, explicit types/units, and the operators in 01. Do not change operator semantics within one schema ID. ID strings/free-text descriptions cannot replace machine decision rules. Reference large data such as scenes/policies through existing ArtifactRef and include it in hazard/validation dependency graphs.

## 4. APIs and state changes

Use common request header `CellCall{context:base.CallContext#1,cell_id:Name#2,expected_cell_revision:uint64?#3}`. Mutation-body field numbers start at 1 for each method in the order listed below, with call first. `base.CallContext.expected_revision` is absent; do not duplicate object-specific expected_* fields. Queries require neither key nor CAS. Follow authentication/current access→same-key identical business payload→new-request CAS order.

| RPC | Input (body) → output | Primary effect |
|---|---|---|
| Cell.Open | CellHello → CellSession | CellSession=`session_id#1,base_session_id#2,peer_id#3,manifest_hash#4`; schema/membership negotiation, no motion |
| Cell.Inspect | call → CellContext | Query current state/block reasons |
| Cell.Evaluate | call, operation_intent:base.Intent, parent:PermitParent → EvaluationSet | EvaluationSet=`evaluations:ConditionEvaluation[]#1,cell:CellRef#2`; decision lookup, not a permit |
| Cell.StartRun | call, run_id:Id, envelope_digest:Digest, purpose:Purpose, budget_unit:BudgetUnit, budget_limit:uint64, origin:StartOrigin → StartAttempt | Initial StartAttempt; no related cases allowed. Check scheduler role/scope |
| Cell.GetStartAttempt | call, attempt_id:Id → StartAttempt | Retrieve PENDING/ARMING/STARTED/REJECTED |
| Cell.BeginPartAttempt | call, run_id:Id, mandate_id:Id, expected_budget_revision:uint64 → PartAttempt | Record unique part attempt/ordinal and budget consumption in the same commit |
| Cell.SubmitOperation | call, request:base.SubmitOperation, parent:PermitParent, part_attempt_id:Id?, expected_run_revision:uint64?, expected_case_revision:uint64? → base.Receipt | Bind base T1+current conditions+cell permit. Run/executor requires run revision; RECOVERY requires case revision |
| Cell.Hold | call, scope_ids:Name[], reason:CellReason, condition_ids:Name[] → CellContext | Authorized general hold request; escalate to latched according to hazard/intervention |
| Cell.ClearTransientBlock | call, block_ids:Id[], evidence_ids:Id[] → CellContext | Clear only the named blocks while the same condition/continuity evidence remains currently valid |
| Cell.OpenCase | call, type:CaseType, scopes:Name[], procedure:ArtifactRef, lead:Name, operation_ids:Id[], material_ids:Id[] → InterventionCase | Atomic case creation and block/fence/mandate handling |
| Cell.RecordProcedure | call, expected_case_revision:uint64, record:ProcedureRecord → InterventionCase | Check actor/procedure/evidence/state transition. Recording API has no native side effects |
| Cell.SetRecoveryPlan | call, case_id:Id, expected_case_revision:uint64, plan:ArtifactRef → InterventionCase | Validate/pin permitted steps/guards/actors/visit limits |
| Cell.PrepareRestart | call, case_revisions:CaseRevision[], target_run_id:Id, restart_plan:ArtifactRef → RestartPreparation | RestartPreparation=`preparation_id:Id#1,target:CellRef#2,status:PreparationState#3,clearance:Clearance?#4,reason:CellReason#5`; fence→revalidation→same-epoch clearance |
| Cell.GetRestartPreparation | call, preparation_id:Id → RestartPreparation | Read-only progress/evidence retrieval |
| Cell.PrepareClose | call, case_revisions:CaseRevision[], evidence_ids:Id[] → Clearance | REMAIN_OUT_OF_SERVICE only; verify closure conditions/restrictions to retain without a run/plan |
| Cell.RestartRun | call, run_id:Id, clearance_id:Id, expected_run_revision:uint64 → StartAttempt | New OPERATOR intent; atomic mandate/cases/clearance commit after Arm acks |
| Cell.CloseWithoutRestart | call, case_revisions:CaseRevision[], clearance_id:Id → CellContext | Non-operational closure; retain latched out-of-service block |
| Cell.RecordChange | call, change:ChangeRecord → ChangeRecord | CAS for impact/actual application/qualification state. Physical changes use separate guarded procedures |
| Cell.RegisterQualification | call, qualification:Qualification → Qualification | Check authority, mandatory evidence, scope, schema; no arbitrary risk-value defaults |
| Cell.GetCase | call, case_id:Id → InterventionCase | Read-only |
| Cell.GetMaterial | call, material_id:Id → MaterialState | Read-only support/location claims and provenance |

PreparationState=PENDING_FENCE,REVALIDATING,READY,REJECTED. The same Cell.StartRun/RestartRun key returns the same attempt. Distinguish following an attempt's state from a new start intent. CREATE/READY may later become REJECTED after invalidation and does not imply native completion.

RegisterQualification accepts human review records, but field validation alone does not create a test pass. Require procedures verifying evidence-artifact scope, authenticity, authority, and version. Reporting an external change through RecordChange immediately invalidates affected guards; do not treat already-changed equipment as “still a draft.”

The server derives required scope/resource/condition sets from validated definitions/envelopes/operation profiles. Callers cannot omit conditions by sending empty conditions or narrower scopes. Required sets must match permit evaluations/scopes; omissions are INVALID_INPUT. Repeated cell/operation/digest/grant/Host values in CellCall/base request/permit must all match. Start budgets cannot exceed envelope/work-order limits. This revision provides no API to change existing run limits; additional scope follows the explicit new-run rules in 02.

In Cell.SubmitOperation, nested base request.context must match outer CellCall.context for session_id/call_id/request_key and omit expected_revision. P maps expected_run_revision to internal base T1 RunView CAS. Do not call the public base RPC again and accept twice. Independent RECOVERY slots check expected_case_revision and plan/step/visit-state CAS in the same T1. Recovery steps belonging to a run also check run CAS. Identical recovery slot/intent retrieves the existing operation even with a different key.

Require expected_cell_revision for permission/condition-expanding StartRun/RestartRun/SubmitOperation/ClearTransientBlock/SetRecoveryPlan/PrepareRestart/PrepareClose/CloseWithoutRestart/qualification activation/change application. Check object revisions as listed. Restriction-increasing actions, such as HOLD or reports of actual external-state changes, must not be lost because of stale UI revisions. If RecordProcedure/RecordChange reports actual access/manual manipulation/configuration application/protective reactions, first record those facts and required latches using valid source/IDs. If the intended step-change CAS fails, create no newly permitted state; return STALE_REVISION in CellError with recorded evidence/case IDs. Distinguish acceptance of trusted factual reports from business-step promotion.

PartAttempt CONFIRMED_COMPLETED means P validated terminal evidence for the processing scope defined by the envelope. BT termination alone cannot create it; do not display “good part” without quality-inspection evidence. Preserve original-operation uncertainty, failure, and human-intervention history even when aggregate state changes.

Operator stop intent through Cell.Hold is LATCHED and revokes the mandate. Only validated condition definitions and system/executor normal waiting/query-reconciliation paths may create TRANSIENT. Caller reason values cannot downgrade protective stops/interventions to transient. Execute/confirm physical stopping/cancel separately through the operation's existing RequestCancel and profile-specific local paths.

## 5. Internal Host protocol

| RPC | Input → output | Atomic rule |
|---|---|---|
| CellHost.Inspect | `call` → HostCellState | Read-only. HostCellState=`cell:CellRef#1,host_boot_id:Id#2,block_ids:Id[]#3,pending_operation_ids:Id[]#4,pending_permit_ids:Id[]#5,definition_digest:Digest#6` |
| CellHost.FenceCell | `call, target:CellRef, block_ids:Id[], invalidation_id:Id` → FenceReceipt | Durable maximum epochs, fencing old unentered permits, ack. Receipt=`target#1,invalidation_id#2,host_boot_id#3,delivery_journal_id#4,seq#5` |
| CellHost.ArmCell | `call, attempt_id:Id, target:CellRef, clearance_ids:Id[], block_ids_to_clear:Id[]` → ArmReceipt | Exact current epochs/events and readiness conditions; specified blocks only. ArmReceipt=`attempt_id#1,target#2,host_boot_id#3,delivery_journal_id#4,seq#5` |
| CellHost.Prepare | `call, base_request:base.PrepareOperation, permit:DispatchPermit` → base.Receipt | Bind profile/member/conditions/permit to base PREPARED. No native production call |
| CellHost.Authorize | `call, base_request:base.AuthorizeDispatch, permit:DispatchPermit` → base.Receipt | In one gate: current epoch/permit/local-condition checks, consumption+SEND_ENTERED+native submission |

H stores maximum cell/scope epochs separately from grants/fences. Old Arm/Fence or full-vector snapshots cannot regress epochs/blocks. Arm cannot clear new-incident blocks; insufficient evidence returns an error instead of ack. P does not commit a mandate if only some Hosts are armed. Already-armed Hosts still lack new permits and cannot start production.

FenceReceipt/ArmReceipt are **RX gate records**. Native-queue clearance, physical stopping, access, and personnel checks need other evidence. Check them separately in readiness/clearance/operation preconditions when required. Uncertainty after CellHost.Authorize retains base SC02 rules.

Explicit cancellation follows existing target operation/cancel_id and only permits the profile's stop/cancel scope. Cancellation APIs cannot bypass into reset, new motion, or arbitrary program execution. This extension provides no general public native safety-reset/muting methods. Where needed, separately specify validated site procedures/native functional paths and record their observations/actions.

## 6. Start and restart sequences

Initial: `StartRun key → P StartAttempt → H ArmCell(current epoch) → all acks/current revision recheck → P mandate+run commit → per-operation permits → H native gate`.

Restart: `case REVALIDATING → PrepareRestart → new scope epoch/FenceCell → residual-work/current-state/personnel/procedure rechecks → target-epoch Clearance/READY_FOR_RESTART → operator RestartRun → same-epoch ArmCell → P mandate/clearance consumption/target-case closure/run commit`.

Discard clearance after any actual-state/case/scope/qualification/authority/configuration change. Simple record lookup is not a change. RestartRun run ID/envelope/resume plan must match clearance-pinned targets. RestartRun itself creates no new epoch. When closing multiple cases, compare the prepared case-revision set against actual open-scope restrictions and reject omissions.

After P crash, do not automatically revive mandates/permits; follow base restart-reconciliation rules. H crash invalidates old arm/permits. Recheck fences/conditions after journal restoration. On backup rollback, cross-check HostCellState durable maximum epochs with base journals and record a new epoch greater than P/related H maxima. Recovery preparation is incomplete if required Host state cannot be retrieved. Retransmission of clearances/attempts requires the same key and semantic body.

Simple observation-sample refresh/query or progress collecting attempt acks does not increment cell/case revision. Changes affecting decision meaning—configuration/blocks, qualification, participants/procedure steps, physical changes/investigation dispositions—increment the corresponding revision. PrepareRestart's final READY+clearance transaction explicitly states its resulting revisions. If no new incident occurs during Arm preparation, its own ack records must not stale the clearance/attempt.

## 7. Canonical representation, events, and errors

Cell-entity semantic digests use `SHA256(UTF8("RX-CELL-v1\n") || JCS({"entity_type":defined_type_name,"value":object}))`. Follow base field/enum/oneof/optional/uint64/set-ordering rules. Exclude CallContext/transport-retry metadata from business fingerprints; include cell/run/case/parent/scope/intent/target disposition. Same key with different scope/clearance/purpose is CONFLICT.

Commit cell-entity state, base operation/run changes, evidence, budgets, events/outbox in the same P transaction. Do not split out a parallel “authorization DB.” Do not automatically delete unresolved events/cases/budgets/consumption/tombstones. Host permit consumption also shares the native-delivery-journal transaction.

`CellJournalRecord{cursor:base.Cursor#1,body:(base_event:base.Event#2|cell_event:CellEvent#3)}` delivers the installation-wide control ledger in order. CellEvent=`event_id:Id#1,type:CellEventType#2,entity:CellEntity#3`; CellEntity oneof numbers 1–12 follow CellContext,Qualification,RunMandate,DispatchPermit,StartAttempt,InterventionCase,ProcedureRecord,Clearance,MaterialState,ChangeRecord,PartAttempt,RestartPreparation. EventType=CONTEXT_CHANGED,QUALIFICATION_CHANGED,MANDATE_CHANGED,PERMIT_CHANGED,START_CHANGED,CASE_CHANGED,PROCEDURE_RECORDED,CLEARANCE_CHANGED,MATERIAL_CHANGED,CHANGE_RECORDED,PART_ATTEMPT_CHANGED,PREPARATION_CHANGED.

When base_event is present, every field of nested base.Event.cursor must match the outer cursor. Do not count one event twice in the new view. SSE data is always CellJournalRecord JSON; id is base64url JCS of the outer cursor; event is `BASE_EVENT` for a base body or `CELL_EVENT` for a cell body. Read the internal enum for the body's concrete type. View conversion does not change stored evidence/outcomes.

CellJournal.GetSnapshot/Subscribe/GetSnapshotPage use base consistent-cut, exclusive-after, gap, snapshot-expiry, and slow-consumer rules, fixed to `view_id=site-cell-control-v1`. Snapshot entities are a typed oneof of base.EntityView or CellEntity, with the cursor at the same DB cut. In this installation, reject old base Journal streams with UPGRADE_REQUIRED rather than providing them as filtered event streams. Do not skip new events to hide base seq gaps.

CellReason order is NONE(1),NOT_COMMISSIONED,OUTSIDE_ENVELOPE,QUALIFICATION_REQUIRED,CONDITION_FAILED,CONDITION_UNKNOWN,BLOCKED_BY_CASE,STALE_EPOCH,MANDATE_REVOKED,BUDGET_EXHAUSTED,PERMIT_EXPIRED,PERMIT_CONSUMED,CONTINUITY_UNPROVEN,CLEARANCE_STALE,PERSONNEL_UNACCOUNTED,EXTERNAL_RESTRICTION,NATIVE_EFFECT_UNRESOLVED,MIXED_CONFIGURATION,UPGRADE_REQUIRED,STALE_REVISION,KEY_CONFLICT,FORBIDDEN,STORE_FAULT,INVALID_INPUT. 0=UNSPECIFIED. Repeated descriptive strings must not replace codes.

CellError=`code:CellReason#1,cell:CellRef?#2,block_ids:Id[]#3,case_ids:Id[]#4,evidence_ids:Id[]#5,detail:string?#6`. Map input errors to INVALID_ARGUMENT/400, key conflicts to ALREADY_EXISTS/409, revisions to ABORTED/409, condition/epoch/case/budget/compatibility mismatches to FAILED_PRECONDITION/409, authority to PERMISSION_DENIED/403, and storage failure to UNAVAILABLE/503. A plain transport timeout implies neither business failure nor restart permission; retrieve receipts by same-key lookup/retransmission.

## 8. Roles and UI contract

| Role/situation | Display | Permitted actions |
|---|---|---|
| General observation | Actual equipment/process, current run, validated scope, block reasons, unknowns | Queries. READY does not allow direct native execution |
| Production operator | Selected recipe/item, permitted attempts/actual counts, current conditions, start scope | Initial start, pause/stop requests, notification acknowledgement |
| Intervention/recovery lead | Related cases/personnel, entry procedures, material, unresolved native effects, revalidation items | Role-appropriate procedure records, plans, restart preparation/new start |
| Engineer/validation/release | Configuration/changes, function allocation, validation evidence/open items/impact | New drafts/reviews/qualification registration. Not command-bypass authority |
| Automatic scheduler | Explicit orders/recipes/budgets/initial-start scope | Permitted initial StartRun. No RestartRun after intervention/safety stops |

UI wording distinguishes meanings such as “Start conditions need checking,” “Stop requested,” “Site procedure confirmation required,” “Ready to restart production,” and “Previous operation outcome unknown.” `ARMED`, `fence ack`, or `GOOD` alone must not display “safe,” “access permitted,” or “torque may be released.” Do not combine ordinary notification dismissal and reset/start into one button action. Do not hide stopping/help paths because production authorization is lacking.

HTTP operational APIs under `/api/cell/v1/cells/{cell_id}` map `inspect`, `evaluate`, `start`, `restart`, `start-attempts/{id}`, `part-attempts`, `operations`, `cases`, `cases/{id}`, `cases/{id}/records`, `recovery-plans`, `restart-preparations`, `restart-preparations/{id}`, `close-preparations`, `changes`, `qualifications`, `materials/{id}`, and explicit `hold`, `clear-transient`, `close-without-restart` paths to their RPCs. close-preparations maps to PrepareClose. Reads use GET; mutations use POST; reject path/body ID mismatches. Provide cursor/SSE at snapshot/events under `/api/cell/v1/journal` with the CellJournalRecord encoding above.

Evaluate accepts complex Intent/parent input as a **read-only POST** exception, creating neither state nor permits. Simple ID lookups use GET. The HTTP gateway binds authenticated user sessions to base sessions/roles; actor/session values in request bodies are not trusted identity evidence.

## 9. Impact on the existing two images

rx-platform adds cell context/condition evaluators, mandates/permits/cases/changes, business/API logic, and the same storage transactions. rx-solutions adds cell-aware Host gates, device-specific condition/function/recovery profiles, UI, and site packages. Declare SDK/driver/transitive dependencies in selected device profiles and image configuration; no particular manufacturer's package is mandatory under the common contract.

Do not operate mixed base-only P/H/executor/UI and cell-enforced releases. Reject paths exposing base Operation.Submit/Workflow.StartRun or direct Host native writes externally to bypass cell policy. Necessary base read APIs may remain, but control-ledger subscriptions and production starts must explicitly support the new contract. If actual SDK/ROS accepts writes outside RX, profiles must address the validation scope for native authority, isolation, and safety functions.

Validate the new image's source manifest, both protocol manifests, profile/schema/qualification caches, Host journals/membership, device/network permissions, and software-ready-without-action conditions together. Automatic command publishing, torque initialization/shutdown, or internal retries by upstream packages outside the gate are nonconforming. Improve startup boundaries/bindings rather than removing included packages, and block those executions until validated.
