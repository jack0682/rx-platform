# Counterexample Review, Validation Obligations, and Requirements Traceability

Scope: RX Contract v1.0 · **Documentary event-trace results**. These are not execution tests, physical tests, or TLC model-checking results.

## 1. Review method and environmental assumptions

P=platform, H=Host, E=executor, D=device. P/H may stop at any time; messages may be lost, duplicated, or delayed. After a native command starts, a crashed process's DB transaction does not roll back device effects. Normal recovery assumes persistent journals survive; journal loss/backup rollback are treated separately.

Each trace distinguishes “what P knows” from “what D may actually have done.” Documentary decisions review the states, evidence, and blocking required by the proposed rules for that trace. They are not presented as proofs enumerating all possible execution orders.

## 2. Normal, loss, and restart traces

| Case | Event order and each actor's knowledge | Normative result/rejected alternative |
|---|---|---|
| SC01 Finite trajectory | ① E submits activation A/slot S ② P stores O and intent in T1 ③ H stores native goal G and PREPARED ④ H sends G after SEND_ENTERED ⑤ H stores D's G result and postcondition observations ⑥ P stores evidence/SUCCEEDED/event in T2 | E looks up P's result to branch. Success concerns joints only; grip/material success needs separate evidence. Reject completion based only on BT success. I01·04·07 |
| SC02 Lost native response | ① H SEND_ENTERED ② D may have executed ③ Response path lost ④ P times out but does not know H/D results ⑤ Resubmitting the same key returns O ⑥ Retrieve H receipt and G result, or retain uncertainty | RECONCILING/UNKNOWN; quarantine resources. Current idle alone establishes neither success nor non-execution. Reject automatic repetition of the same SDK call. I02·03·04 |
| SC03-A P restart | ① P stores T1/pending dispatch ② H PREPARED or SEND_ENTERED ③ Only P restarts ④ New P restores ledger and the same O ⑤ Query H receipt/generation/unacknowledged evidence ⑥ Check authority handover/current conditions | PREPARED may proceed after review with a new grant; SEND_ENTERED permits only queries. Reject unconditional ledger-outbox replay that produces work. Preserve historical O. I01·03·05 |
| SC03-B H restart | ① H stores SEND_ENTERED ② Crash just before/after native call ③ P and D survive ④ H resumes journal ⑤ Discard old grants/samples ⑥ Query native result/current state | Even a crash before the call does not establish non-execution once SEND_ENTERED exists. Retain UNKNOWN instead of repeating. Sacrifice some availability to prevent automatic duplicate native calls. I03·06 |
| SC03-C D/controller restart | ① O has success/progress history ② Only D restarts ③ H loses current sample/mode/native-cache continuity ④ New device-session boundary ⑤ Recheck firmware/mode/residual commands/physical state ⑥ Re-evaluate readiness | Preserve historical durable success; invalidate current predicates. Lost result cache for an in-progress O yields UNKNOWN. Without boot observations, profiles declare “detection not guaranteed.” I04·09 |
| SC05 E restart | ① P stores A/S→O ② D completes/P commits result ③ E restarts without receiving response ④ E queries Run/Checkpoint/A/S ⑤ Retrieves existing O ⑥ Proposes next step from that result | Do not use a new tick UUID as a new production key. The next loop iteration of the same node requires a new-visit activation. I02·11 |

## 3. Races, observations, and other operation kinds

| Case | Event order and each actor's knowledge | Normative result/rejected alternative |
|---|---|---|
| SC04 Cancel/completion race | ① P records cancel intent ② D already succeeded at G or is stopping ③ Cancel response arrives first ④ G result arrives later ⑤ Evaluate compatibility by profile/correlation ⑥ Check quiescence/material handover separately | Cancel acceptance does not establish CANCELED. Success evidence can coexist with a cancel request; contradictory terminal native results yield DISPUTED. Resource release is separate. I04·08 |
| SC06 Observation disconnection | ① Last sample seq=120 GOOD ② Only observation path disconnects; command path remains live ③ H repeatedly reads cache ④ Original seq/age unchanged ⑤ Freshness expires ⑥ Block readiness/postcondition evaluation | Do not refresh age from cache-function invocation time. RUNNING/UNKNOWN oscillation without new evidence is not an outcome. I04·09 |
| SC07 Device mode | ① Record ReadyPose request in T1/H ② Mode service accepts ③ Observe ModeStatus=ReadyPose ④ Actual pose/stability postconditions remain unsatisfied ⑤ Wait for required observations ⑥ Satisfy rule or timeout/reconcile | Reject mapping service acceptance to posture success. Mode-only and pose-establishment rules belong to different profiles. I04·10 |
| SC08 Mobile/leader stream | ① session/grant/ticket OPEN ② Accept increasing-seq samples ③ Source disconnects or owner changes ④ ticket/lease/deadman expires ⑤ Block new samples + execute local expiry reaction ⑥ Confirm residual commands/support before handover | Do not replay slow buffered samples under new authority. Separate acceptance of a zero-velocity command from actual stopping. Do not apply the same torque-off to humanoids. I05·06·08 |
| SC09 Driver startup/shutdown | ① Profile declares init/activate/destructor effects ② Check required support/mode/calibration ③ Record lifecycle operation ④ Enter native initialization/shutdown ⑤ During shutdown, discover unconfirmed support handover ⑥ Defer graceful driver shutdown | Reject automatic healthcheck→activate or shutdown timeout→torque-off promotion. Separate physical protection against forced power loss needs subsequent validation. I01·08·10·12 |
| SC10 Package only | ① Selected profile package included ② Actual controller/mode or calibration mismatch ③ Produce admission finding ④ No native dispatch ⑤ Correct required profile/calibration ⑥ Admit after new validation | Default inclusion does not mean completed support/execution authorization. Do not activate profiles not actually validated. I10 |
| SC14 Arm/gripper sharing JTC | ① A requests arm trajectory, resource=controller/J ② B requests gripper on the same J ③ T1 detects conflict ④ B BUSY ⑤ Confirm A result/resource release ⑥ Newly admit B, or design one compound trajectory from the outset | Reject parallelism inferred from different logical arm/gripper names. A compound trajectory must be one validated native goal. I05·08 |

## 4. Protocol and storage traces

| Case | Event order and each actor's knowledge | Normative result/rejected alternative |
|---|---|---|
| SC11 Slow subscriber | ① subscriber cursor=500 ② Producer continues committing ③ Buffer limit reached ④ Explicitly terminate subscriber ⑤ Resubscribe from 500, preventing duplicate application ⑥ Outside retention, use same-cut snapshot then through+1 | Reject control-producer blocking, infinite buffers, and silent event drops. Distinct from telemetry coalescing. I07·09 |
| SC12 Mixed schemas/languages | ① PeerHello hash negotiation ② Block commands without common hash ③ Check requiredness/presence/units under common schema ④ Expand defaults/JCS ⑤ Compare P/H digests ⑥ No native call on mismatch | Reject hashing raw proto bytes or language map-iteration order. Reject execution after converting unknown enums to 0. I02·04·10 |
| SC13-A T1 storage failure | ① Validate key/intent ② T1 commit fails or return is lost ③ Block normal dispatch ④ Look up same key ⑤ Retrieve receipt if commit confirmed ⑥ STORE_FAULT if unconfirmable | Reject UI issuing production again with a new key after an error response. I01·02·07 |
| SC13-B H journal failure | ① H PREPARED ② SEND_ENTERED commit fails ③ No normal native call ④ Separate path for required protective reaction ⑤ Compare journal after recovery | Reject ignoring DB write failure to “move first, record later.” I01·12 |
| SC13-C Storage failure after D completion | ① SEND_ENTERED/actual action ② H or P write fails before result persistence ③ P cannot establish completion ④ Attempt local protection/evidence retention ⑤ T2 if result recoverable ⑥ UNKNOWN/UNRESOLVED otherwise | Reject recording actual completion as cancellation or automatically re-executing. I03·04·07·12 |
| SC13-D Rollback of both histories | ① Restore old backups after historical action ② Restore procedure changes generation ③ Block old-outbox republication ④ Compare peer watermarks ⑤ Investigate current state/material/residual commands ⑥ commissioning/recovery disposition | Complete detection of concealed simultaneous rollback is outside the guarantee. Reject “a backup always permits automatic resumption.” I03·05·09 |

## 5. Additional counterexamples and rules strengthened during review

| Counterexample | Strengthening |
|---|---|
| Replayed lease renewals keep extending expiry | Increasing renew_seq; duplicates return original response. Prepare validity is fixed separately from lease |
| A stream frame delayed in the network is applied as fresh immediately after reconnection | Host-issued expiring ticket, source boot/seq, latest-only sample. Source-side stale-data checks also required |
| Historical success changed to failure because the device later rebooted | Manage historical outcome separately from current-observation validity |
| Support for a gravity-loaded part released after CANCEL based only on a terminal result | Require physical-handover conditions for RELEASED |
| Subscriber filtering creates seq gaps mistaken for packet loss | v1 streams the entire fixed authorized view. Specify per-view cursors and absence of filtering |
| P and Host share a DB volume and both modify the same state | Only P writes the business ledger; Host has a separate receipt journal |
| Multiple native calls silently retried inside one Adapter function | Decompose RX composites into child operations. Declare partial-outcome limits of OEM native composites |
| Old command-key namespace changes after a new role/auth token | Separate peer/client identity from credential lifetime |
| Valid T1-ticket seq=10 arrives after T2-ticket seq=11 | Reject seq=10 using a high-watermark across the whole control session/source boot. Ticket renewal does not reset sequence |
| Thread stalls just after grant check, then resumes after handover to a new owner | Serialize checks, SEND_ENTERED, native submission, and revocation through one gate. No handover while an entered call remains |
| Late Authorize resurrects an operation immediately after PREPARED is voided | Single CAS between VOIDED_BEFORE_SEND and SEND_ENTERED; P outbox also races NEW→VOIDED/EMIT_ENTERED |
| Late unused activation slot arrives after pause/next step | Atomically check run/checkpoint/activation/authority eligibility in T1. Allow only lookup of existing mappings |

## 6. Subsequent formal-verification scope

Minimum TLA+ model variables are `P_intents, P_outbox, P_results, H_receipts, H_evidence, native_calls, grants, fences, messages, crashes`. Transitions are Admit, Prepare, Authorize, NativeCall, RecordEvidence, CommitResult, Cancel, Expire, CrashP/H/D, Deliver/Drop/Duplicate, and Restore.

Safety properties to check are I01–I12. In particular, `production_native_calls[operation] ≤ 1` applies to **production invocations without automatic retries for finite actions/state writes/mode and startup transitions**. Model explicit cancellation as `cancel_native_calls[cancel_id] ≤ 1`, streams as increasing session/source_seq, and I12 local protective commands as separate incidents. Do not interpret hardware repetition/continuous control inside one native call as “one physical change.”

Liveness to check: “when persistent storage, communication, observations, and ownership recover and required evidence becomes available, a RECONCILING operation can reach an outcome or explicit UNRESOLVED disposition.” Do not attempt to prove automatic success or unconditional progress where devices provide no results. Both TLC bounded model checking and conformance testing of the actual contract implementation remain subsequent obligations.

## 7. Implementation and site-validation obligations

| Validation ID | Work/required deliverables | Responsible role | Restriction until passed |
|---|---|---|---|
| V01 | Rust/C++ protobuf presence, unknown/duplicate fields, enums, JCS/digest, schema-hash golden fixtures | Platform+integration | Cannot claim protocol conformance on both sides |
| V02 | Inject crashes, lost responses, duplicates, and disk-full before/after T1/T2/T3/Host SEND_ENTERED | Platform+validation | Cannot release restart guarantees |
| V03 | Single writer, fences, leases, renewal replay, authority handover, external-native-client detection | Integration+operations | No automatic authority handover |
| V04 | Actual driver/firmware/calibration/observation/cancel/startup/shutdown tests per selected support ID/mode | Device integration | No production admission for that profile |
| V05 | Verify SC14 shared JTC and bus/space resource conflicts | Robotics+process | Cannot authorize independent parallel control |
| V06 | PLC OEM I/O map, handshake, actual feedback, boot, residual commands, operator-recovery contract | PLC/OEM+process | Cannot activate door/chuck/machining-start bindings |
| V07 | Sample age, deadman, tickets, local stop/support, CPU/GPU load, device-disconnection tests | Control+validation | Cannot guarantee control-stream performance/reactions |
| V08 | Backup/restore, generation changes, journal loss, synchronization, and power-loss storage tests | Deployment+operations | No unattended automatic recovery |
| V09 | Concurrent snapshot writes, pagination expiry, 7-day retention, slow subscribers, storage pressure | Platform+operations | Cannot claim long-term operation/diagnostic conformance |
| V10 | Acceptance of UI/audit for uncertainty, contradictions, interventions, abandonment, current-state rechecks | Product+site operations | Cannot accept operator-recovery procedures |
| V11 | TLA+ bounded safety/liveness and spec-to-implementation comparison | Contract+validation | Cannot claim completed formal verification |

“Responsible role” does not mean an individual has been assigned in an organization. Test durations, cycle counts, and thresholds must be defined in test specifications according to site-profile risks/requirements. Do not invent arbitrary numbers as validation-completion criteria.

## 8. Requirements traceability and design-completion audit

| Requirement/item | Primary document | Status |
|---|---|---|
| Boundaries/responsibilities C01–C08 | 01 §1, 03 §5, 04 | v1.0 design finalized |
| Operation classification/semantics | 01 §2–7 | v1.0 design finalized |
| State/identity/recovery D01–05·08–09 | 01–02 | v1.0 design finalized |
| Order/generation/resubscription/time/buffers D06 | 02 §5–7, 03 §7–8 | v1.0 design finalized |
| Data/transport/compatibility D07·10 | 03 | v1.0 design finalized |
| Counterexamples across models/modes/versions | 04 and SC01–14/A–D here | Documentary trace review performed; not experiments |
| Alternatives/choices/limits/subsequent validation | 05, research report, V01–11 here | Evidence and obligations documented |

The original independent reader review is preserved at the immutable source linked from `review_record.md`. V01–V11 are validation obligations; verify actual fulfillment separately against implementation commits and test evidence. This documentation revision does not claim all tests were executed.

This document is the vendor-neutral documentation revision. Earlier design-audit results in the tables apply only to the contemporaneous scope of the [original document](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/contracts/v1.0/06_scenarios_and_validation.md), and are not execution-test results for this revision.
