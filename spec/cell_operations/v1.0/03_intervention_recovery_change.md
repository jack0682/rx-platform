# Operator Intervention, Recovery, and Change Contract

Normative: RX Cell Operations Contract v1.0. Procedures here describe software-state/record transitions; they do not prescribe the physical sequence of opening doors, isolation, torque, or stopping controls.

## 1. Intervention types and scope

An InterventionCase contains `case_id, revision, cell/scope, cause, procedure_digest, related run/operation/material, participants and lead, state/evidence`. Types are `DIAGNOSTIC_ONLY / PLANNED_ACCESS / FAULT_RECOVERY / MAINTENANCE / CHANGE_REVIEW`.

DIAGNOSTIC_ONLY means reading and reconciling existing states/results without human access or physical manipulation. This label cannot conceal actual manual manipulation or safety reset. Access, manual native control, or external material changes immediately require the appropriate type and latched scope.

Case scope includes related hazard zones, residual native operations, and material support as well as command resources. If impact is unclear, use the entire cell. Overlap with other open cases requires checking common coordination responsibility and permitted actions. Closing one case does not clear another case's protection/blocks.

## 2. State transitions

| State | Records/evidence required for entry | Permissions/restrictions |
|---|---|---|
| OPEN | Cause, affected scope, responsible role, related operations/material | Hold related new production. Queries/local protective reactions allowed |
| CONTAINMENT_PENDING | Prior permit/mandate revocation, fence propagation, stop/support requests/local-reaction records | Fence acknowledgements/command responses alone must not display access permission |
| PROCEDURE_ACTIVE | **Explicit external evidence** that site-procedure entry conditions hold, participants, and work scope | Only site work permitted by that procedure. No normal production permission |
| REVALIDATING | Participant work completion/handover, changed material/equipment/settings, restoration-stage records for access/isolation procedures | Recheck current state, residual commands, safety functions/modes, calibration, zones, and configuration |
| READY_FOR_RESTART | Required reviews/dispositions complete, no unresolved contradictions, evidence that other-case/external restrictions are cleared | Ready for a new start request. No automatic action |
| CLOSED | Accepted new RestartRun or explicit non-operational closure disposition | Only new-mandate scope may execute. Never revive the old mandate |
| ESCALATED | Conflicting evidence, absent required functions/observations, or loss of responsible personnel | Retain related production blocks and required local protection/support procedures |

PROCEDURE_ACTIVE does not certify that RX has established safe human access. It records conditions supplied by validated site procedures and actual protection/isolation functions. Ordinary operator acknowledgement, passwords, or remote video confirmation cannot substitute. Without person/accountability/source information, retain UNKNOWN.

DIAGNOSTIC_ONLY may close through OPEN→REVALIDATING→CLOSED after recovering the original invocation result and evidence of no human access, no **physical changes unexplained by the original invocation and validated normal behavior**, no protective reactions, and no generation loss. Correlated effects of normal picking/movement do not themselves block this path. Only in this case may the original ACTIVE mandate continue after clearing TRANSIENT under 02. Other paths require READY_FOR_RESTART and a new RestartRun.

New loss, person/object changes, or conflicting evidence discovered in any state invalidate prior clearance and return to CONTAINMENT_PENDING or ESCALATED. Advancing a step number alone does not automatically satisfy subsequent conditions.

## 3. People, access, and handover records

ProcedureRecord contains procedure ID/revision, responsible actor, action kind, actual occurrence/confirmation time, cell/scope, physical equipment/personnel/material references, evidence source/value, and the step's validation result. Actions are `ACKNOWLEDGE`, `ENTRY_CONDITIONS_REPORTED`, `WORK_STARTED`, `WORK_FINISHED`, `PERSONNEL_ACCOUNTED`, `ISOLATION_STATE_REPORTED`, `RESET_OBSERVED`, `HANDOVER_ACCEPTED`, and `CONFIGURATION_REPORTED`.

- ACKNOWLEDGE only records that a notification was read; it changes no condition, outcome, access, reset, or start.
- RESET_OBSERVED records an actual safety-function/device reset report or a fact confirmed through a permitted procedure. This recording API does not execute a native reset.
- PERSONNEL_ACCOUNTED is the procedure-defined participant/zone check. An empty list, logout, or lack of response is not evidence that no people are present.
- ISOLATION_STATE_REPORTED references actual device/procedure isolation/restoration facts. Do not represent an RX logical lock as physical LOTO.
- Replacing a responsible person records outgoing/incoming roles and acceptance of remaining work, personnel, isolation, and material state. A case does not expand permission to proceed before handover completes.

When multiple crews/maintenance personnel/operators share a scope, retain individual work/personnel states and overall coordination responsibility. One person's work-completion record does not remove restrictions for others/external locks. Include references/continuity for separate contractor procedures. Specific personnel counts and isolation actions are site-procedure inputs, not arbitrary choices.

## 4. Recovery plans

A RecoveryPlan binds to the case, envelope, procedure revision, current material/device state, and an explicit finite set of permitted steps. Step kinds are `OBSERVE / RECORD_EXTERNAL_ACTION / GUARDED_OPERATION / VERIFY / DISPOSITION`. GUARDED_OPERATION uses existing operation kinds and a new RECOVERY permit.

Required fields per step: preconditions, related scopes, native semantics, completion evidence, failure/stop reactions, next steps, and permitted actors. False production preconditions do not justify omitting all guards in recovery. Specify required alternative guards and actual protective conditions; there is no generic bypass flag.

`(case_id, plan_digest, step_id, visit)` is P's persistent unique execution slot. Bind it to the guarded step's operation/digest in the same T1; submitting the same slot with another request key retrieves only the same operation. Different intent for the same slot conflicts. P creates a next visit only after checking plan-permitted edges/counts, current case state, and prior-step evidence. Callers cannot re-execute merely by increasing a visit number. Apply valid plan/guard changes only after invalidating affected scopes, fencing old unentered permits, and reconciling entered operations.

Declare native resume semantics as `CONTINUE_CORRELATED / RESTART_FROM_ENTRY / NO_RESUME`. CONTINUE_CORRELATED requires the previous native invocation and continuation token/point, effects already performed, and evidence of current pose/material/residual commands. RESTART_FROM_ENTRY may repeat earlier program sections and must not implement automatic retransmission of the original operation. If needed, define a separately validated recovery program/new operation starting from current state. NO_RESUME permits only result investigation/site-procedure reconciliation.

Do not change UNRESOLVED to success. Resuming subsequent production requires records of both the disposition of historical uncertainty and confirmation of current state. Irreversible machining/material changes are not DB rollbacks. Subsequent use/inspection/hold dispositions for a part require explicit results from defined quality/site roles.

## 5. SR01 — Uncertainty and human intervention during material feeding

Assumption: an operation is SEND_ENTERED without a result, leaving material support uncertain. This is not a test that created the situation on actual equipment.

1. P retains the existing operation as UNKNOWN/RECONCILING and holds related subsequent dispatch. Query the original key without a new production call.
2. If queries alone recover evidence, consider DIAGNOSTIC_ONLY. If a person is needed or continuity cannot be confirmed, create a FAULT_RECOVERY case/latched scope and revoke the mandate.
3. Even after H installs a fence, confirm physical stopping/support separately. If access is needed, do not enter PROCEDURE_ACTIVE before obtaining condition/participant evidence required by external procedures/functions.
4. Record operator material repositioning, manual movement, and tool/calibration changes; invalidate related MaterialState/Condition/Qualification.
5. Recheck current state, personnel/zones, native queues, mode/owner, required protective functions, and configuration. Settle the uncertain operation using recovered evidence or record UNRESOLVED disposition.
6. Only when the case is READY_FOR_RESTART and other related restrictions are cleared may a new RestartRun create a new mandate and explicit start/continuation point.

Preparation must finish before pressing explicit restart. A restart click does not turn every remaining check true. The final H gate and actual protective conditions remain required after restart.

## 6. SR02 — Communication, power, and energy loss

Profiles define scope, local reaction, states that must be maintained, and evidence for each loss cause. Do not collapse robot disconnection, observation-only loss, control-source loss, PLC/controller restart, and pressure/power/battery anomalies into one timeout reason.

For stationary models include axis/tool/material retention; for wheeled models include body position/braking/docking coupled with arm motion; for actively balanced models include whole body/support, reaction forces, and recovery space. Native names such as zero velocity, brake release, Damping, and torque disable do not establish a safe state. Determine device-specific energy retention/isolation procedures through function allocation and testing.

Required local reactions must operate without P or storage. On recovery, investigate reactions already performed, record gaps, device generations, current support, and residual commands. Do not reuse normal production permits/mandates after loss that breaks authority/device/physical continuity or causes local protective reactions. Simple observation/response holds with proven continuity follow the TRANSIENT path in 02, but consumed/expired permits are never reused. Graceful runtime-process shutdown also follows function/support conditions; healthcheck failure alone does not justify forced driver restart/torque transitions.

## 7. SR03 — Configuration, policy, and mechanical changes

ChangeRecord records before/after artifact hashes, reason, whether actually applied, affected qualifications/conditions/recovery/functions/tests, review roles, and disposition.

| Change | Impacts requiring review |
|---|---|
| Tool/fixture/calibration/material/load | Paths, seating, support, interference, hazard scope, stop/contact conditions |
| Robot/PLC firmware/controller/native program | Acceptance/result/restart semantics, residual queues, modes, signals |
| Policy/model/controller gains/command schema | Outputs/behavior, ranges, stopping/support, sensors, authority, validation conditions |
| Layout/map/docking/floor/zones | Location/access, hazard impact scope, simultaneous actions, recovery space |
| Communication/Host/OS/CPU/GPU variant | Freshness, latency, deadman, native ownership, common-cause failures |
| Responsible personnel/operational procedures | Work/handover, access, required-presence conditions, intervention/restart |

A design-draft change is not an applied site change. Distinguish actual application as `PROPOSED→IMPACT_REVIEWED→STAGED→APPLIED_UNQUALIFIED→QUALIFIED_ACTIVE`. New STAGED artifacts cannot be selected for execution. Do not attach previous qualification to new configuration before checking affected guards/tests.

Applying changes requires new epochs for affected scopes, fencing old permits, handling entered native work/support, and fence/configuration acks from related Hosts. Quarantine partial Host updates as MIXED_CONFIGURATION. Treat unknown dependencies as whole-cell impact. “Same part number” or “only the checksum file changed” is not evidence of no impact.

## 8. Closure, abandonment, and unfinished work

Case CLOSED does not mean production restart. Non-operational closure records `REMAIN_OUT_OF_SERVICE` and retains a separate latched block. Clearing it requires a new intervention/qualification procedure. Do not forget unresolved physical scopes by simply labeling them “operation canceled.”

A Clearance is a one-use restart-preparation record bound to specific case revisions, cell/scope epochs, evidence, and target disposition. Changes invalidate it. Apply restrictions of multiple cases as a union. Reject related RestartRun if any remain or external isolation/personnel status is unknown.

For non-operational closure, `PrepareClose` issues REMAIN_OUT_OF_SERVICE clearance without requiring a target run or restart plan. Verify evidence of target-case work/personnel handover, current hazard containment/isolation state, and restrictions to retain, bound to case revisions in the same transaction. Closure cannot ignore ongoing personnel work or unconfirmed external state. CloseWithoutRestart atomically consumes this clearance, closes target cases, and creates a separate out-of-service LATCHED block; it does not call ArmCell.
