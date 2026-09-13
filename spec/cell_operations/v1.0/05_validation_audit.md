# Counterexamples and Validation Obligations

Status: 2026-09-14 vendor-neutral documentation revision. The following are documentary event traces, not product-execution, formal model-checking, physical, or safety-function test results.

## 1. Invariants to verify

| ID | Contract requirement |
|---|---|
| OI01 | An admitted operation's envelope/qualification/dependency hashes and current scopes match. |
| OI02 | Do not turn required-condition UNKNOWN into PASS or silently ignore unknown schemas. |
| OI03 | Native production/setup/recovery writes pass through the cell-aware gate for the corresponding purpose/parent. |
| OI04 | Revoked mandates are not restored, distinct from clearing transient normal waits. |
| OI05 | Pending StartAttempt/Arm acks/clearance do not themselves authorize native execution/access. |
| OI06 | Operation permits bind ID/content/Host/epoch/purpose and are not reused after consumption/expiry. |
| OI07 | Support-reducing operations on the same material conflict through common resources; current mutual-support PASS does not permit simultaneous release. |
| OI08 | Do not permit production restart while omitting related open cases/personnel/external restrictions. |
| OI09 | Treat notification acknowledgement, reset observation, work completion, condition revalidation, and new start as distinct actions. |
| OI10 | Related changes invalidate prior conditions/qualification/clearance and require new-epoch synchronization. |
| OI11 | Controller/Host/P/executor restarts/backup rollback do not automatically execute old mandates/permits. |
| OI12 | Do not claim actual safety-function performance from epistemic consistency, data GOOD, or UI display. |
| OI13 | Preserve authorized control-stream sessions/epochs/sequences and actual native-application order. |
| OI14 | Failure of DB/communication/normal production conditions does not block required local protective reactions. |
| OI15 | Distinguish part-attempt, node-activation, and operation budgets; restarts do not refund consumption. |
| OI16 | Distinguish support-package inclusion/document approval from actual commissioning. |
| OI17 | Host cell/scope epochs never regress, and old Arm/Fence requests never clear blocks. |
| OI18 | Expose the interval between P invalidation and H application; allocate required physical reactions to local functions. |

## 2. Normal, hold, and restart traces

Notation: P=platform, H=Host, E=executor, D=device, U=operator. Distinguish durable records from current observations; do not infer facts unknown to the source.

| Case | Event sequence | Contract conclusion |
|---|---|---|
| CO01 Normal automatic repetition | ① Qualified envelope+new StartRun ② H Arm ack ③ P mandate commit ④ Create 1 part_attempt/consume 1 budget unit ⑤ Each of that part's 6 nodes has T1/permit/result ⑥ Create next part | 6 nodes do not consume 6 material-budget units. Each native action has separate results/conditions; no extra human clicks inside a normal run. OI01·03·15 |
| CO02 Lost response/recovered normal effects | ① Normal native picking effect O/G ② Only response lost ③ P TRANSIENT/same-key lookup ④ Confirm same G result and explained object movement/support ⑤ Confirm no intervention/protective reaction and same-generation continuity ⑥ Clear transient and continue to next node | No original native repeat call. Normal physical movement alone does not categorically prohibit the diagnostic path. Existing ACTIVE mandate may continue. OI04·06 |
| CO03 Uncertainty/human intervention (SR01) | ① O SEND_ENTERED ② Queries cannot confirm support/result ③ FAULT_RECOVERY with latch/fence/revocation ④ External evidence for site-entry procedure ⑤ Record manual actions/revalidate ⑥ UNRESOLVED disposition or recovered result | Do not fabricate prior success. Only prepared epoch/clearance and new RestartRun permit production. OI04·08·09 |
| CO04 Reset alone | ① Safety stop revokes mandate ② D reports reset ③ U acknowledges notification ④ Some current conditions PASS ⑤ No StartIntent | No new mandate/permit. Do not map reset or popup dismissal to start. OI05·09 |
| CO05 Prevent self-invalidation on restart | ① Cases REVALIDATING/e7 ② PrepareRestart fences e8 ③ Recheck residual commands/state ④ Atomically commit READY transition and post-transition case revisions/e8 clearance ⑤ U RestartRun/same-e8 Arm ⑥ P consumes clearance+closes cases+creates mandate | Prohibit creating clearance(e7) then changing to e8, or staling case revision through the READY transition itself. OI05·10·17 |
| CO06 Partial Arm | ① H1 Arm ack ② H2 no response ③ P attempt ARMING ④ Old-request retransmission or timeout | No new mandate commit/production permits. H1 gate readiness alone cannot produce work. OI03·05 |
| CO07 Normal WAIT versus protective reaction | ① Normal material wait FAIL/UNKNOWN ② Conditions recover without change/intervention / or ②′ local protective reaction occurs ③ Recheck classification | Only the first path clears transient. ②′ requires latch/revocation then new start. Do not classify solely from the word “wait.” OI02·04 |

## 3. Race, personnel, material, and change traces

| Case | Event sequence | Contract conclusion |
|---|---|---|
| CO08 Delayed invalidation | ① P issues permit ② P discovers condition loss and stores latch/fence ③ Old permit reaches H during communication delay ④ H local guard or actual protective path reacts ⑤ Fence installed | Do not claim immediate physical blocking from P DB time. Old-epoch entry prohibited after fence; required reaction time is a local-function obligation. OI17·18 |
| CO09 Old-epoch replay | ① H installs A8/B6 ② Delayed A7/B6 Fence or old Arm arrives ③ Check local map | Reject STALE; no map-snapshot replacement or deletion of new blocks. OI17 |
| CO10 Multiple cases/operators | ① C1/U1 and C2/U2 share scope ② U1 finishes/C1 prepared ③ RestartRun lists only C1 ④ Check C2/external restrictions | Reject while C2 remains. Handover records/empty lists do not establish U2's departure. OI08·09 |
| CO11 Recovery motion required | ① Production guard unmet/case active ② Plan requires explicit observation/control operation ③ Evaluate purpose RECOVERY+case/plan/step+alternative guards ④ H gate | Do not skip all production guards. No motion outside permitted recovery steps. Required local protection remains active. OI03·14 |
| CO12 Tool-change/start race (SR03) | ① Prepared with clearance/tool vA ② Actual vB-application report races StartAttempt ③ Same cell/case revision CAS ④ Update impact closure/epochs/qualification | Cannot finally commit mandate against old target. Partial Host changes yield MIXED_CONFIGURATION. OI01·10 |
| CO13 Release of both supports | ① G and C each support PASS ② G release and C release submitted to different Hosts ③ P CAS on same material/support resource ④ Only one request reserves ⑤ Decide next action after result/current-support checks | Native actions do not simultaneously release using each other's stale PASS. Separate mechanical/functional validation of maintained support is required. OI07 |
| CO14 Shared JTC | ① Arm/gripper are logically distinct operations ② Same controller resource ③ Permit-reservation race | Serialize or use one validated native composite trajectory. Different names do not permit parallelism. base SC14+OI03 |
| CO15 Independent scopes | ① RunA uses A; RunB uses validated independent B ② Only A latched ③ Compute A closure/increment Aepoch ④ Retain B vector/conditions | Validated independent B may continue if unaffected. Unknown mapping blocks the entire cell. OI01·10·17 |
| CO16 Incorrect clearance reuse | ① Prepare e8/runA/planA/cases{C1,C2} ② Request runB, planB, or omit C2 ③ Compare targets/revisions/body | Reject. Authorization is not a general token for similar work. OI05·06·08 |

## 4. Device differences, loss, and version traces

| Case | Event sequence | Contract conclusion |
|---|---|---|
| CO17 Driver startup return | ① Guarded lifecycle starts ② Source-internal torque-enable attempt may fail/be unconfirmed ③ Callback SUCCESS ④ No actual torque/support evidence | Related conditions UNKNOWN; no promotion to production readiness. Do not exaggerate source success strings into completion evidence. OI02·16 |
| CO18 Policy-based control mode | ① Software startup or ReadyPose/policy request ② Native command publishing possible ③ Observe mode/status separately from actual pose/support | A binding whose output boundary bypasses the cell gate is nonconforming. Required packages may be installed without automatic activation before validation. OI03·12·13 |
| CO19 Wheeled/legged models (SR02) | ① Base position or whole-body/energy conditions lost during work ② Required local response ③ P/RPC may be absent ④ Reconcile current location/support/residual commands | zero velocity/Damping/torque-off names do not guarantee stopping/support. Require function allocation/validation by model/mode/load/environment. OI12·14 |
| CO20 Safety mirror GOOD | ① Mirror packet freshly GOOD ② Actual protective function may be disabled/faulty ③ Check whole-function evidence/current signal scope | Do not treat data quality as safety-function performance. Without required evidence, qualification/conditions are unsatisfied. OI02·12 |
| CO21 Storage failure | ① P/H cannot persist ② Block new normal permits/production dispatch ③ Local protection required ④ Preserve/recover available evidence | Protection does not wait for DB commit. Record failure does not cancel prior physical effects. OI06·14 |
| CO22 Backup/reboot | ① P backup at e5, H at e9 ② After P restore, inspect H/compare journals ③ New epoch/fence above maximum ④ Review state/qualification/cases | Do not unilaterally impose e6 or replay old permits. Unreadable peer history means preparation is incomplete. OI11·17 |
| CO23 Old-version bypass | ① Cell-enforced deployment ② Base-only UI/Host directly invokes StartRun/Authorize ③ Check mandatory feature/endpoint policy | UPGRADE_REQUIRED/authority error, no fallback native action. Do not silently skip cell events in the old journal. OI03·16 |
| CO24 Run-restart budget | ① Run budget 10, 4 part attempts consumed ② New mandate after uncertainty/intervention ③ Retrieve remaining budget of same run ④ Continue same part or create part 5 | Preserve consumption 4. Nodes of the same part consume no additional part unit. Unknown outcomes are not refunded to permit unlimited production. OI15 |
| CO25 Graceful shutdown/driver destruction | ① Shutdown requested ② Driver destructor may disable torque ③ Material/gravity support still required ④ Check lifecycle conditions | Shutdown/container-restart policy must not bypass mechanical-support conditions. Forced power loss belongs to separate physical-protection scope. OI03·14·16 |
| CO26 Non-operational closure | ① Abandon operation without a restart plan ② PrepareClose verifies work/personnel/restriction evidence ③ REMAIN_OUT_OF_SERVICE clearance ④ Atomically close/create separate latch | No fictitious run/plan required. No Arm call/production permission. Preserve other-case restrictions. OI05·08 |
| CO27 Recovery retry with changed key | ① case/plan/step/visit1 binds O ② Submit same slot with new key ③ Query persistent unique slot | Same intent retrieves O; different intent conflicts. visit2 checks plan edges/evidence/state/limits. OI03·06 |
| CO28 Actual-change report from stale UI | ① Actual manual manipulation/access occurs after clearance preparation ② Report with old case revision ③ First record trusted fact/evidence/latch ④ Return step-CAS conflict | Actual change is not discarded because revision is old. No newly permitted transition; restart preparation invalidated. OI08·09·10 |

## 5. Original sources, facts, and validation scope

Original research sources for CO17, CO18, and CO14 are preserved in the [immutable original](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/cell_operations/v1.0/05_validation_audit.md). Current tables apply those failure types as documentary counterexamples to vendor-neutral configurations. They are not validation of new driver sources or physical equipment. Remaining entries are hypothetical event traces applied to the contract's asynchronous/human/change model. They review “what outcome the rules require in a trace,” not how actual equipment behaved.

Static-document-review evidence concerns whether conditions, transitions, APIs, and roles consistently require these conclusions. The validations below must establish whether implementations satisfy them. Candidate formal-model variables are cell/scope epochs, block sets, mandates, permits, cases, clearances, budgets, messages, native-entry counts, and source validity. Formal-verification terms safety/liveness are distinct from machine functional-safety certification.

## 6. Subsequent implementation/site-validation obligations

| Validation | Required evidence/decision | Scope/responsibility |
|---|---|---|
| OV01 Condition evaluator | Golden fixtures for UNKNOWN/FAIL/ANY/ALL, contradictions, empty conditions, unit/schema/age | Platform/Host contract implementation |
| OV02 Permit/fence races | Fault tests at each commit boundary for issuance, consumption, revocation, Arm, epoch rollback, lost acks; native-entry traces | Platform/Host; extends base V01–03 |
| OV03 Restart | PrepareRestart target epochs, clearances, case revisions, all-Host barriers, late events, multiple-case traces | Platform/operations |
| OV04 Native recovery semantics | Per-controller continue/from-entry/none; effects already performed, current support, residual commands | OEM/robot/device integration |
| OV05 Material support | Detection limits/loss behavior for support/seating/grasp, dual-release conflicts, support resources, actual protection | Mechanics/robotics/PLC/installation |
| OV06 Safety functions | Actual sensor/logic/output/drive paths required by H01–H06; reaction times, required performance/achievement evidence, faults/environments/loads | Manufacturing team/OEM/functional-safety/site validation |
| OV07 Human intervention | Access/isolation, personnel, handover/external restrictions, reset/start, site-UI user validation | Site operations/installation |
| OV08 Change/deployment | Partial updates, new policies/tools, source/Host changes, backup rollback, rejection of mandatory-cell-capability bypass | Release/platform/solutions |
| OV09 Selected device configurations | Actual mode/driver/stream/startup/shutdown and external direct-native-write boundaries per selected profile | Each device integration owner |
| OV10 Budgets/results | PartAttempt versus activation/operation, identical keys, restart remaining budgets, uncertainty/defect/cancel history | Platform/product |
| OV11 Formal verification/conformance | OI01–18 in bounded models, corresponding implementation traces, schema/manifest/generated-output conformance | Contract/validation |

This table defines validation obligations. Check actual fulfillment separately against implementation commits and execution evidence; these were not rerun by this documentation revision. Do not fill actual repetition counts, thresholds, or durations without inputs. Block unfulfilled site combinations as NOT_COMMISSIONED/REVALIDATION_REQUIRED. Distinguish finalizing common software-design documents from functional/site validation of a particular delivery.

## 7. Distinguishing the first-edition audit from current review

First-edition N01–N06 completion-audit/independent-review results are preserved in the [original dated 2026-09-10](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/cell_operations/v1.0/05_validation_audit.md). Do not relabel its models, support obligations, or source inventory as validation of the current neutral configuration.

The vendor-neutral revision changed device-support policy/application examples while preserving common semantic, authorization, and recovery invariants and validation obligations. The [revision record](revision_2026-09-14.md) records impacts, counterexamples, hashes, and compatibility scope. Actual model, signals, mechanics, personnel, protective performance, and timing values remain inputs for profiles not yet verified; do not fill them by guessing.
