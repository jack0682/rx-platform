# 데이터·API·기존 계약 연결·운영 화면

규범: RX 셀 운영 계약 v1.0. base=`rx.contract.v1`, extension=`rx.cell.v1`. 표의 `name:type#n`은 field 번호, `?`는 optional, `[]`는 repeated다. 번호는 이 설계의 규범이며 생성 코드/빌드는 아직 없다.

## 1. 공통 표현과 협상

Id/Name/Digest/TimePoint/CallContext/ArtifactRef/Observation/Evidence/Receipt 및 RPC 기본 오류는 base 규칙을 따른다. UTF-8, proto3 optional, 미지 enum/field·중복 field·NaN·부정확한 단위 거부, JCS 의미 투영도 유지한다. enum 0은 UNSPECIFIED이며 유효한 업무 값이 아니다. 아래 열거 순서대로 1부터 번호를 부여한다.

CellHello는 `base_manifest_hash:Digest#1, cell_manifest_hash:Digest#2, peer_id:Name#3, base_session_id:Id#4, cell_definition_digest:Digest#5, shared_clock_id:string#6`이다. peer는 두 계약과 CellDefinition을 명시적으로 지원해야 한다. cell_manifest_hash는 이 폴더의 protocol_manifest.json JCS bytes에 대한 SHA-256이며 01–04 규범 문서 hash와 base manifest hash를 포함한다. CellSession.session_id는 협상 기록 ID이고, CellCall/context 및 RunMandate.executor_session_id에는 **base_session_id**를 사용한다. 협상 기록은 인증된 같은 peer/base session에 묶이며 base session 종료 시 무효다.

이 feature가 활성화된 셀의 native 쓰기는 **cell-aware Host gate만** 통과한다. base Host.AuthorizeDispatch를 네트워크에 남겨 permit 없이 실행하는 하향 경로는 금지한다. base 구현 함수를 내부 호출할 수는 있지만 호출자가 강제 cell validation을 건너뛸 수 없어야 한다. 단순 config의 `optional_permit`은 허용하지 않는다.

기존 최대 message/batch·snapshot·consumer buffer·retention 제한은 base를 따른다. envelope는 permit TTL, clearance TTL, preparation/Arm 대기 상한, 조건별 최대 age·취득 오차와 근거를 필수로 지정한다. permit/clearance 유효 시각은 관련 evidence의 유효 한계를 넘을 수 없고, Host grant expiry도 별도 검사한다. 준비 대기 timeout은 실패/재조정 이유이며 native stop 완료로 해석하지 않는다. 기본 안전 수치를 임의 제공하지 않는다.

## 2. 핵심 타입

| 타입 | 필드 |
|---|---|
| ScopeEpoch | `scope_id:Name#1, epoch:uint64#2`. scope_id 오름차순 정렬, 중복/0/후퇴 거부 |
| CellRef | `cell_id:Name#1, cell_epoch:uint64#2, scopes:ScopeEpoch[]#3` |
| CellContext | `cell_id:Name#1, revision:uint64#2, cell_epoch:uint64#3, scopes:ScopeEpoch[]#4, definition:ArtifactRef#5, envelope:ArtifactRef#6, qualification_id:Id?#7, mode:OperatingMode#8, blocks:Block[]#9, open_case_ids:Id[]#10, commissioning:Commissioning#11` |
| Qualification | `qualification_id:Id#1, revision:uint64#2, envelope:ArtifactRef#3, state:QualificationState#4, evidence:ArtifactRef[]#5, dependency_hashes:Digest[]#6, reviewed_by:Name[]#7, limitations:ArtifactRef#8` |
| ConditionEvaluation | `condition_id:Name#1, condition_revision:uint64#2, verdict:Verdict#3, reason:CellReason#4, evidence_ids:Id[]#5, evaluated_at:TimePoint#6, valid_until:TimePoint#7, cell:CellRef#8` |
| Block | `block_id:Id#1, kind:BlockKind#2, reason:CellReason#3, scope_ids:Name[]#4, condition_id:Name?#5, case_id:Id?#6, created_revision:uint64#7` |
| RunMandate | `mandate_id:Id#1, run_id:Id#2, cell:CellRef#3, envelope_digest:Digest#4, actor:Name#5, origin:StartOrigin#6, purpose:Purpose#7, state:MandateState#8, budget:RunBudget#9, executor_session_id:Id#10, start_attempt_id:Id#11, restart_plan:ArtifactRef?#12` |
| RunBudget | `unit:BudgetUnit#1, limit:uint64#2, consumed:uint64#3, revision:uint64#4`. limit>0, consumed≤limit |
| PartAttempt | `part_attempt_id:Id#1, run_id:Id#2, ordinal:uint64#3, material_id:Id?#4, revision:uint64#5, disposition:PartDisposition#6`. 식별 근거와 물리 확인은 MaterialState에서 구별한다 |
| DispatchPermit | `permit_id:Id#1, operation_id:Id#2, intent_digest:Digest#3, cell:CellRef#4, envelope_digest:Digest#5, qualification_id:Id#6, qualification_revision:uint64#7, purpose:Purpose#8, parent:PermitParent#9, grant:base.Grant#10, host_boot_id:Id#11, issued_at:TimePoint#12, expires_at:TimePoint#13, conditions:ConditionEvaluation[]#14, state:PermitState#15` |
| PermitParent | oneof `mandate_id:Id#1` 또는 `recovery:RecoveryStepRef#2`. RecoveryStepRef=`case_id:Id#1,plan:ArtifactRef#2,step_id:Name#3,visit:uint64#4` |
| StartAttempt | `attempt_id:Id#1, run_id:Id#2, status:StartStatus#3, target:CellRef#4, expected_cell_revision:uint64#5, case_revisions:CaseRevision[]#6, clearance_ids:Id[]#7, mandate_id:Id?#8, reason:CellReason#9`. CaseRevision=`case_id:Id#1,revision:uint64#2` |
| InterventionCase | `case_id:Id#1, revision:uint64#2, cell:CellRef#3, type:CaseType#4, state:CaseState#5, procedure:ArtifactRef#6, lead:Name#7, participants:Name[]#8, operation_ids:Id[]#9, material_ids:Id[]#10, record_ids:Id[]#11, block_ids:Id[]#12` |
| ProcedureRecord | `record_id:Id#1, case_id:Id#2, case_revision:uint64#3, action:ProcedureAction#4, actor:Name#5, scope_ids:Name[]#6, occurred_at:base.UtcTime#7, evidence_ids:Id[]#8, assertions:ArtifactRef#9` |
| Clearance | `clearance_id:Id#1, case_revisions:CaseRevision[]#2, cell:CellRef#3, evidence_ids:Id[]#4, prepared_at:TimePoint#5, valid_until:TimePoint#6, disposition:ClearanceDisposition#7, consumed_by:Id?#8, target_run_id:Id?#9, envelope_digest:Digest#10, restart_plan:ArtifactRef?#11`. RESTART에는 target_run_id와 restart_plan 필수 |
| MaterialState | `material_id:Id#1, revision:uint64#2, identity_basis:IdentityBasis#3, identity_evidence:Id[]#4, location_claim:Name?#5, supports:SupportClaim[]#6, scope:CellRef#7`. SupportClaim=`holder:Name#1,verdict:Verdict#2,evidence_ids:Id[]#3,valid_until:TimePoint#4` |
| ChangeRecord | `change_id:Id#1, revision:uint64#2, cell:CellRef#3, before:ArtifactRef[]#4, after:ArtifactRef[]#5, state:ChangeState#6, affected_ids:Name[]#7, evidence_ids:Id[]#8, applied_by:Name?#9` |

OperatingMode=SETUP,AUTOMATIC,RECOVERY,MAINTENANCE. Commissioning=NOT_COMMISSIONED,COMMISSIONED,REVALIDATION_REQUIRED. QualificationState=DRAFT,IN_REVIEW,QUALIFIED,SUSPENDED,RETIRED. Verdict=PASS,FAIL,UNKNOWN. BlockKind=TRANSIENT,LATCHED. StartOrigin=OPERATOR,BATCH_SCHEDULER. Purpose=PRODUCTION,SETUP,RECOVERY. MandateState=ACTIVE,REVOKED,EXHAUSTED,CLOSED. BudgetUnit=PART_ATTEMPT,OPERATION_COUNT. PartDisposition=IN_PROGRESS,CONFIRMED_COMPLETED,REJECTED,UNRESOLVED,NOT_PROCESSED. PermitState=ISSUED,CONSUMED,VOIDED,EXPIRED. StartStatus=PENDING,ARMING,STARTED,REJECTED. 

CaseType/CaseState/ProcedureAction/ChangeState는 03의 열거 순서를 따른다. ClearanceDisposition=RESTART,REMAIN_OUT_OF_SERVICE. IdentityBasis=VERIFIED_SENSOR,TRACKED_SLOT,HUMAN_DECLARED,UNKNOWN_IDENTITY. 모든 counter는 base JSON 표현에 따라 10진 string이다. 사람 이름은 immutable actor identity와 별도 표시 metadata로 둔다.

MaterialState는 multi-support claim이며 machine safety certificate가 아니다. PartAttempt는 **run 소유**의 허용 시도 예산 단위다. 물체가 바뀌면 같은 part_attempt_id로 사실을 숨기지 않고 material identity/처분·변경 기록과 연결한다. PRODUCTION의 SubmitOperation에는 같은 run의 part_attempt_id가 필수이고 여러 node는 이를 공유한다. 재시작 때는 현재 mandate의 restart_plan이 기존 part의 해당 continuation을 허용하는지 검사한다. SETUP에는 part_attempt_id가 absent이며 OPERATION_COUNT를 사용한다. RECOVERY는 mandate 대신 RecoveryStepRef를 필수로 사용하고 03의 영속 unique visit slot에 연결한다.

## 3. artifact schema의 필수 내용

아래 schema ID는 내용 형식의 식별자다. 해당 schema를 이해하지 못하면 digest 검사가 성공해도 업무에 사용하지 않는다. content-addressed artifact는 두 image에 같은 revision으로 배포하고 arbitrary URL fetch를 실행 중 허용하지 않는다.

| schema ID | 필수 semantic fields |
|---|---|
| rx.cell-definition.v1 | cell ID, device/Host/native endpoint membership, control resource·material/support resource, scopes/zones, 의존 edge, 외부 보호 기능과 source 역할, 충돌/독립성 근거 |
| rx.operating-envelope.v1 | 01 §3 전체 사용 한계, profile/recipe/tool/지그/교정/소재·환경 hash, purpose/mode·budget·source/role, 조건·case/recovery 허용, timing 및 근거, hazard/function/test 참조 |
| rx.condition-definition.v1 | condition ID/revision, 적용 purpose/step·ADMISSION/MAINTAINED/WAIT_TARGET, typed grammar tree, sources/units/quality·age/충돌 규칙, 영향 scope, TRANSIENT/LATCHED 및 현지 반응 참조 |
| rx.risk-function-map.v1 | hazard ID·작업/모드/노출/사건/피해, 사용범위, 조치/function ID, 책임·요구 성능 출처·검증·잔존 위험·미확인 항목 |
| rx.safety-function-spec.v1 | 01 §7의 trigger·전체 경로 할당·상태/유지·시간·성능/근거·고장·reset/start·검증·제약 |
| rx.recovery-plan.v1 | case/절차/envelope 범위, 03 §4의 finite step 종류·edge·visit 한도·대체 guard·native continuation·실패/중단·evidence·actor |
| rx.restart-plan.v1 | target run/recipe/envelope, 원 run/operation·현재 material/part_attempt 연결, 새 activation 또는 CONTINUE_CORRELATED의 지점/근거, native 재개 의미, 예산 원장, 필요한 재시작 조건 |
| rx.procedure-assertions.v1 | 절차 단계와 주체, 실제 외부 기능/행위·scope/인원·물리 대상, source/권한·유효성, 주장/미확인·근거 참조 |

조건 grammar는 유한 tree, 명시 타입/단위, 01의 연산만 허용한다. 같은 schema ID에서 연산 의미를 바꾸지 않는다. ID 문자열·free text 설명은 machine 판정 규칙을 대신할 수 없다. scene·policy 등 큰 데이터는 기존 ArtifactRef로 참조하고 위험/검증 의존 graph에 포함한다.

## 4. API와 상태 변경

공통 request header `CellCall{context:base.CallContext#1,cell_id:Name#2,expected_cell_revision:uint64?#3}`를 쓴다. mutation body는 method별로 아래 나열 순서의 field 번호 1부터 가지며 첫 field는 call이다. `base.CallContext.expected_revision`은 absent로 하고 객체별 expected_*와 중복시키지 않는다. 조회에는 key/CAS를 요구하지 않는다. 인증·현재 접근권→같은 key의 동일 업무 payload→신규 요청의 CAS 순서를 따른다.

| RPC | 입력(body) → 출력 | 주효과 |
|---|---|---|
| Cell.Open | CellHello → CellSession | CellSession=`session_id#1,base_session_id#2,peer_id#3,manifest_hash#4`; schema/membership 협상, motion 없음 |
| Cell.Inspect | call → CellContext | 현재 상태/차단 이유 조회 |
| Cell.Evaluate | call, operation_intent:base.Intent, parent:PermitParent → EvaluationSet | EvaluationSet=`evaluations:ConditionEvaluation[]#1,cell:CellRef#2`; 판단 조회이며 permit 아님 |
| Cell.StartRun | call, run_id:Id, envelope_digest:Digest, purpose:Purpose, budget_unit:BudgetUnit, budget_limit:uint64, origin:StartOrigin → StartAttempt | 최초 StartAttempt, 관련 case 없어야 함. scheduler 역할/범위 검사 |
| Cell.GetStartAttempt | call, attempt_id:Id → StartAttempt | PENDING/ARMING/STARTED/REJECTED 회수 |
| Cell.BeginPartAttempt | call, run_id:Id, mandate_id:Id, expected_budget_revision:uint64 → PartAttempt | 유일 part 시도/ordinal과 예산 소비를 같은 commit에 기록 |
| Cell.SubmitOperation | call, request:base.SubmitOperation, parent:PermitParent, part_attempt_id:Id?, expected_run_revision:uint64?, expected_case_revision:uint64? → base.Receipt | base T1+현재 조건+셀 permit 연결. run/executor에는 run revision, RECOVERY에는 case revision 필수 |
| Cell.Hold | call, scope_ids:Name[], reason:CellReason, condition_ids:Name[] → CellContext | 권한 있는 일반 보류 요청; 위험/개입에 따라 latched로 승격 |
| Cell.ClearTransientBlock | call, block_ids:Id[], evidence_ids:Id[] → CellContext | 동일 condition/연속성 근거가 현재도 유효할 때만 해당 block 해제 |
| Cell.OpenCase | call, type:CaseType, scopes:Name[], procedure:ArtifactRef, lead:Name, operation_ids:Id[], material_ids:Id[] → InterventionCase | case 생성·block/fence/mandate 처리 atomic |
| Cell.RecordProcedure | call, expected_case_revision:uint64, record:ProcedureRecord → InterventionCase | actor·절차·evidence·상태 transition 검사. record API에 native 부작용 없음 |
| Cell.SetRecoveryPlan | call, case_id:Id, expected_case_revision:uint64, plan:ArtifactRef → InterventionCase | 유효 step/guard/actor/visit 한도 검증·고정 |
| Cell.PrepareRestart | call, case_revisions:CaseRevision[], target_run_id:Id, restart_plan:ArtifactRef → RestartPreparation | RestartPreparation=`preparation_id:Id#1,target:CellRef#2,status:PreparationState#3,clearance:Clearance?#4,reason:CellReason#5`; fence→재확인→동일 epoch clearance |
| Cell.GetRestartPreparation | call, preparation_id:Id → RestartPreparation | read-only 진행/근거 회수 |
| Cell.PrepareClose | call, case_revisions:CaseRevision[], evidence_ids:Id[] → Clearance | REMAIN_OUT_OF_SERVICE 전용, run/plan 없이 종료 조건·남길 제한 확인 |
| Cell.RestartRun | call, run_id:Id, clearance_id:Id, expected_run_revision:uint64 → StartAttempt | OPERATOR 새 의도, Arm ack 뒤 mandate/cases/clearance 원자 commit |
| Cell.CloseWithoutRestart | call, case_revisions:CaseRevision[], clearance_id:Id → CellContext | 비운전 종료, latched out-of-service block 유지 |
| Cell.RecordChange | call, change:ChangeRecord → ChangeRecord | 영향·실제 적용·검증 상태 CAS. 물리 변경은 별도 guarded 절차 |
| Cell.RegisterQualification | call, qualification:Qualification → Qualification | 권한·필수 근거·범위·schema 검사, 위험값 임의 default 없음 |
| Cell.GetCase | call, case_id:Id → InterventionCase | read-only |
| Cell.GetMaterial | call, material_id:Id → MaterialState | read-only 지지·위치 주장과 provenance |

PreparationState=PENDING_FENCE,REVALIDATING,READY,REJECTED. 같은 Cell.StartRun/RestartRun key는 동일 attempt를 반환한다. attempt 상태를 따라가는 조회와 새 시작 intent를 구별한다. CREATE/READY가 나중에 invalidation으로 REJECTED가 될 수 있으며 native 완료 의미를 갖지 않는다.

RegisterQualification은 사람의 검토 기록을 받지만 field validator만으로 시험 통과를 생성하지 않는다. evidence artifact의 범위·진위·권한·버전 확인 절차를 요구한다. 외부 변경을 RecordChange로 보고한 즉시 영향 guard를 invalidate하며, 이미 실제로 바뀐 장비를 ‘아직 초안’으로 처리하지 않는다.

필요한 scope/resource/조건 집합은 검증된 definition/envelope/operation profile에서 서버가 도출한다. caller가 빈 conditions나 좁은 scope를 보내 조건을 생략할 수 없다. 필요 집합과 permit의 평가/범위가 일치해야 하며 누락은 INVALID_INPUT이다. CellCall·base request·permit에 반복된 cell/operation/digest/grant/Host 값도 모두 같아야 한다. 시작 budget은 envelope/작업지시가 허용한 범위를 넘을 수 없다. 본 판에서 기존 run limit 수정 API는 없으며 추가 범위는 02의 새 명시적 run 규칙을 따른다.

Cell.SubmitOperation의 nested base request.context는 외부 CellCall.context와 session_id/call_id/request_key가 같고 expected_revision은 absent여야 한다. P는 expected_run_revision을 내부 base T1의 RunView CAS로 매핑한다. 공개 base RPC를 재호출해 두 번 접수하지 않는다. RECOVERY의 독립 slot은 expected_case_revision·plan/step/visit 상태 CAS를 같은 T1에서 검사한다. run에 속한 recovery step이면 run CAS도 함께 검사한다. 동일 recovery slot/같은 의도는 key가 바뀌어도 기존 operation 회수다.

조건/권한을 확대하는 StartRun/RestartRun/SubmitOperation/ClearTransientBlock/SetRecoveryPlan/PrepareRestart/PrepareClose/CloseWithoutRestart/qualification 활성/변경 적용에는 expected_cell_revision을 필수로 한다. 객체 revision도 해당 표대로 검사한다. HOLD나 실제 외부 상태 변화의 보고처럼 제한을 늘리는 행위는 오래된 UI revision 때문에 누락하지 않는다. RecordProcedure/RecordChange가 실제 접근·수동 조작·설정 적용·보호 반응을 보고하면 유효한 source/ID로 그 사실과 필요한 latch를 먼저 기록한다. 의도한 단계 변경의 CAS가 실패하면 새 허용 상태는 만들지 않고 CellError에 기록된 evidence/case ID와 STALE_REVISION을 반환한다. 신뢰된 사실 보고의 수용과 업무 단계 승격은 구별한다.

PartAttempt의 CONFIRMED_COMPLETED는 envelope가 정의한 해당 처리 범위의 종료 근거를 P가 검증했음을 뜻한다. BT 종료만으로 만들지 않으며 품질 검사 근거가 없는 경우 ‘양품’으로 표시하지 않는다. 원래 operation의 불명·실패·사람 개입 이력은 aggregate 상태가 바뀌어도 보존한다.

Cell.Hold의 operator 중단 의도는 LATCHED이며 mandate를 철회한다. TRANSIENT는 검증된 조건 정의와 시스템/실행기의 정상 대기·조회 조정 경로만 만들 수 있다. caller의 reason 값으로 보호정지/개입을 transient로 낮추지 않는다. 물리 정지/취소는 해당 operation의 기존 RequestCancel 및 profile별 현지 경로를 별도 실행·확인한다.

## 5. Host 내부 protocol

| RPC | 입력 → 출력 | 원자 규칙 |
|---|---|---|
| CellHost.Inspect | `call` → HostCellState | read-only. HostCellState=`cell:CellRef#1,host_boot_id:Id#2,block_ids:Id[]#3,pending_operation_ids:Id[]#4,pending_permit_ids:Id[]#5,definition_digest:Digest#6` |
| CellHost.FenceCell | `call, target:CellRef, block_ids:Id[], invalidation_id:Id` → FenceReceipt | durable 최대 epoch·미진입 옛 permit 봉인·ack. Receipt=`target#1,invalidation_id#2,host_boot_id#3,delivery_journal_id#4,seq#5` |
| CellHost.ArmCell | `call, attempt_id:Id, target:CellRef, clearance_ids:Id[], block_ids_to_clear:Id[]` → ArmReceipt | exact 현재 epoch/사건과 준비 조건, specified block만. ArmReceipt=`attempt_id#1,target#2,host_boot_id#3,delivery_journal_id#4,seq#5` |
| CellHost.Prepare | `call, base_request:base.PrepareOperation, permit:DispatchPermit` → base.Receipt | profile/member/조건·permit과 base PREPARED 연결. native production 호출 없음 |
| CellHost.Authorize | `call, base_request:base.AuthorizeDispatch, permit:DispatchPermit` → base.Receipt | 동일 gate에서 current epoch/permit/현지조건 검사, 소비+SEND_ENTERED+native 제출 |

H는 grant/fence와 별도로 cell/scope 최대 epoch를 저장한다. old Arm/Fence나 전체 vector snapshot이 epoch·block을 후퇴시킬 수 없다. Arm은 새 incident block을 지울 수 없으며 증거가 불충분하면 ack 대신 오류다. 일부 Host만 arm이면 P는 mandate를 commit하지 않는다. 이미 arm된 Host에도 아직 새 permit가 없으므로 생산을 시작할 수 없다.

FenceReceipt/ArmReceipt는 **RX gate 기록**이다. native queue 해소·물리 정지·접근·인원 확인은 다른 evidence다. 그러한 조건이 필요한 경우 readiness/clearance/operation precondition에서 별도로 검사한다. CellHost.Authorize 이후 불명은 base SC02 규칙을 유지한다.

명시적 취소는 기존 target operation/cancel_id를 따라 처리하되 profile의 stop/cancel 범위만 허용한다. 취소 API로 reset·새 motion·임의 프로그램 실행을 우회할 수 없다. native safety reset/뮤팅 command는 이 extension에서 일반 공개 메서드로 제공하지 않는다. 필요한 경우 검증된 현장 절차·native 기능 경로를 별도 명세하고 그 관측/행위를 기록한다.

## 6. 시작·재시작 시퀀스

최초: `StartRun key → P StartAttempt → H ArmCell(현재 epoch) → 전체 ack/현재 revision 재검사 → P mandate+run commit → operation별 permit → H native gate`.

재시작: `case REVALIDATING → PrepareRestart → 새 scope epoch/FenceCell → 잔류작업·현재상태·인원/절차 재확인 → target epoch Clearance/READY_FOR_RESTART → operator RestartRun → 같은 epoch ArmCell → P mandate/clearance소비/대상case종료/run commit`.

clearance 발급 후 실제 상태·case·scope·qualification·권한·구성 변화가 생기면 폐기한다. 단순 기록 조회는 변화가 아니다. RestartRun의 run ID/envelope/재개 계획은 clearance에 고정된 대상과 같아야 한다. RestartRun 자체는 새 epoch를 만들지 않는다. 복수 case를 함께 닫을 때 준비 대상 case revision 집합과 실제 열린 scope 제한의 집합을 대조해 누락을 거부한다.

P crash면 mandate/permit를 자동 살리지 않고 base 재시작 재조정 규칙을 따른다. H crash면 old arm/permit 유효성 없음. journal 복원 후 fence와 조건을 새로 확인한다. 백업 rollback에서는 HostCellState의 durable 최대 epoch와 base journal을 교차 확인하고, P/관련 H 최대값보다 큰 새 epoch를 기록한다. 필요한 Host 상태를 회수할 수 없으면 복원 준비가 끝난 것이 아니다. clearance/attempt를 재전송할 때 동일 key·동일 semantic body만 허용한다.

관측 sample의 단순 갱신·조회·attempt의 ack 수집 진행 자체는 cell/case revision을 올리지 않는다. 구성/차단·자격·참여자/절차 단계·물리 변경/조사 처분처럼 판정 의미를 바꾸는 변경이 해당 revision을 올린다. PrepareRestart의 최종 READY+clearance transaction은 그 결과 revision을 명시한다. Arm 준비 중 새 사건이 없다면 자기 ack 기록 때문에 clearance/attempt가 stale가 되지 않아야 한다.

## 7. canonical·사건·오류

cell entity의 의미 digest는 `SHA256(UTF8("RX-CELL-v1\n") || JCS({"entity_type":정의된타입명,"value":객체}))`다. base field/enum/oneof/optional/uint64/집합 정렬 규칙을 따른다. CallContext·전송 retry metadata는 업무 fingerprint에서 제외하고 cell/run/case/parent/scope/의도/목표 처분은 포함한다. 같은 key 다른 scope/clearance/purpose는 CONFLICT다.

cell entity 상태+base operation/run 변화+evidence+budget+event/outbox는 P의 동일 transaction에 기록한다. 새로운 병렬 ‘허가 DB’로 분리하지 않는다. 미결 사건·case·budget·소비/tombstone은 자동 삭제하지 않는다. Host permit 소비도 native delivery journal과 같은 transaction이다.

`CellJournalRecord{cursor:base.Cursor#1,body:(base_event:base.Event#2|cell_event:CellEvent#3)}`로 설치 전체 제어 원장을 순서대로 전달한다. CellEvent=`event_id:Id#1,type:CellEventType#2,entity:CellEntity#3`이고 CellEntity는 CellContext,Qualification,RunMandate,DispatchPermit,StartAttempt,InterventionCase,ProcedureRecord,Clearance,MaterialState,ChangeRecord,PartAttempt,RestartPreparation 순의 oneof 번호 1–12다. EventType=CONTEXT_CHANGED,QUALIFICATION_CHANGED,MANDATE_CHANGED,PERMIT_CHANGED,START_CHANGED,CASE_CHANGED,PROCEDURE_RECORDED,CLEARANCE_CHANGED,MATERIAL_CHANGED,CHANGE_RECORDED,PART_ATTEMPT_CHANGED,PREPARATION_CHANGED. 

base_event가 들어 있으면 nested base.Event.cursor와 outer cursor는 모든 필드가 같아야 한다. 새 view의 같은 event 한 개를 두 번 계수하지 않는다. SSE data는 항상 CellJournalRecord JSON이며 id는 outer cursor의 base64url JCS, event는 base body일 때 `BASE_EVENT`, cell body일 때 `CELL_EVENT`다. body의 구체 type은 내부 enum을 읽는다. 실제 stored evidence/outcome는 view 변환으로 변경하지 않는다.

CellJournal.GetSnapshot/Subscribe/GetSnapshotPage는 base와 같은 consistent cut·exclusive after·갭·snapshot expiry·느린 소비자 규칙을 쓰며 `view_id=site-cell-control-v1`로 고정한다. Snapshot의 entities는 base.EntityView 또는 CellEntity의 typed oneof이고 cursor까지 같은 DB cut이다. 이 설치에서는 옛 base Journal stream을 사건 필터로 제공하지 않고 UPGRADE_REQUIRED로 거부한다. 일부 새 사건을 skip해 base seq gap을 감추지 않는다.

CellReason은 NONE(1),NOT_COMMISSIONED,OUTSIDE_ENVELOPE,QUALIFICATION_REQUIRED,CONDITION_FAILED,CONDITION_UNKNOWN,BLOCKED_BY_CASE,STALE_EPOCH,MANDATE_REVOKED,BUDGET_EXHAUSTED,PERMIT_EXPIRED,PERMIT_CONSUMED,CONTINUITY_UNPROVEN,CLEARANCE_STALE,PERSONNEL_UNACCOUNTED,EXTERNAL_RESTRICTION,NATIVE_EFFECT_UNRESOLVED,MIXED_CONFIGURATION,UPGRADE_REQUIRED,STALE_REVISION,KEY_CONFLICT,FORBIDDEN,STORE_FAULT,INVALID_INPUT 순이다. 0=UNSPECIFIED다. 반복 가능한 설명 문자열을 code 대용으로 사용하지 않는다.

CellError=`code:CellReason#1,cell:CellRef?#2,block_ids:Id[]#3,case_ids:Id[]#4,evidence_ids:Id[]#5,detail:string?#6`이다. 입력 오류는 INVALID_ARGUMENT/400, key 충돌 ALREADY_EXISTS/409, revision ABORTED/409, 조건·epoch·case·예산·호환 불일치 FAILED_PRECONDITION/409, 권한 PERMISSION_DENIED/403, 저장 불능 UNAVAILABLE/503으로 매핑한다. 단순 transport timeout은 업무 실패/재시작 허가를 뜻하지 않으며 동일 key 조회/재전송으로 receipt를 회수한다.

## 8. 역할과 UI 계약

| 역할/상황 | 화면에 보여줄 것 | 허용되는 행위 |
|---|---|---|
| 일반 관찰 | 실제 장비/공정·현재 run·검증 범위·차단 사유·미확인 | 조회. READY 표시로 native 직접 실행 불가 |
| 생산 작업자 | 선택 recipe/품목·허용 시도/실적·현재 조건·시작 범위 | 최초 시작, pause/stop 요청, 알림 확인 |
| 개입/복구 책임 | 관련 case/인원·진입 절차·소재·미결 native·재확인 항목 | 역할에 맞는 절차 기록·계획·재시작 준비/새 시작 |
| 엔지니어/검증·릴리스 | 구성/변경·기능 할당·검증 근거/미결/영향 | 새 초안·검토·qualification 등록. 명령 우회권 아님 |
| 자동 scheduler | 명시된 주문·recipe·예산·첫 시작 허용 범위 | 허용된 최초 StartRun. 개입/안전정지 뒤 RestartRun 금지 |

UI 문구는 ‘시작 조건 확인 필요’, ‘정지 요청됨’, ‘현장 절차 확인 필요’, ‘생산 재시작 준비됨’, ‘이전 작업 결과 불명’처럼 의미를 구별한다. `ARMED`, `fence ack`, `GOOD`만으로 ‘안전’, ‘출입 가능’, ‘토크 해제 가능’을 표시하지 않는다. 일반 알림 닫기와 reset/start를 같은 버튼 동작으로 묶지 않는다. 정지/도움 경로를 생산 허가 부족 때문에 숨기지 않는다.

HTTP 운영 API는 `/api/cell/v1/cells/{cell_id}` 아래 `inspect`, `evaluate`, `start`, `restart`, `start-attempts/{id}`, `part-attempts`, `operations`, `cases`, `cases/{id}`, `cases/{id}/records`, `recovery-plans`, `restart-preparations`, `restart-preparations/{id}`, `close-preparations`, `changes`, `qualifications`, `materials/{id}`와 명시적 `hold`, `clear-transient`, `close-without-restart` 경로를 대응 RPC에 매핑한다. close-preparations는 PrepareClose다. 읽기는 GET, mutation은 POST이며 path/body ID 불일치는 거부한다. cursor/SSE는 `/api/cell/v1/journal`의 snapshot/events로 제공하고 위 CellJournalRecord encoding을 따른다.

복잡한 Intent/parent를 받는 evaluate는 **read-only POST** 예외이며 상태·permit를 생성하지 않는다. 단순 ID 조회는 GET이다. HTTP gateway는 인증된 사용자 세션을 base session과 역할에 연결하며 body에 적힌 actor/session을 신원 증거로 신뢰하지 않는다.

## 9. 기존 두 이미지에 대한 영향

rx-platform에는 셀 context/condition evaluator·mandate/permit/case/change·업무/API·같은 저장 transaction이 추가된다. rx-solutions에는 cell-aware Host gate, 장비별 조건/기능/복구 profile, UI와 현장 패키지가 추가된다. 자사 SDK·DHI·OpenManipulator·AI Worker·AI Sapiens 및 전이 의존성 기본 포함은 유지한다.

base-only P/H/executor/UI와 cell-enforced release를 혼합해 동작시키지 않는다. base Operation.Submit/Workflow.StartRun 및 직접 Host native 쓰기를 외부에 공개해 셀 정책을 우회하는 경로는 거부한다. 필요한 base 읽기 API는 유지할 수 있으나 제어 원장 구독과 생산 시작은 새 계약을 명시적으로 지원해야 한다. 실제 SDK/ROS가 RX 밖의 쓰기를 받는다면 native authority/격리/안전기능의 검증 범위를 profile에서 해결해야 한다.

새 image의 source manifest, 두 protocol manifest, profile/schema/qualification cache, Host journal/membership, 장치·네트워크 권한과 software-ready 무동작 조건을 함께 검증한다. 상위 패키지의 자동 command publishing·토크 초기화/종료·내부 재시도가 이 gate 밖에서 발생하면 적합하지 않다. 포함 패키지를 없애는 대신 기동 경계·binding을 보완하고 해당 실행은 검증 전 차단한다.
