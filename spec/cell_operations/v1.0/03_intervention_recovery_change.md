# 작업자 개입·복구·변경 계약

규범: RX 셀 운영 계약 v1.0. 여기의 절차는 소프트웨어 상태/기록의 전이이며 실제 문 개방·격리·토크·정지 조작 순서를 지시하지 않는다.

## 1. 개입 유형과 범위

InterventionCase는 `case_id, revision, cell/scope, 원인, procedure_digest, 관련 run/operation/material, 참여자와 책임자, 상태/근거`를 갖는다. 유형은 `DIAGNOSTIC_ONLY / PLANNED_ACCESS / FAULT_RECOVERY / MAINTENANCE / CHANGE_REVIEW`다.

DIAGNOSTIC_ONLY는 사람 접근·물리 조작 없이 기존 상태/결과를 읽어 조정하는 경우다. 이 유형을 붙여 실제 수동 조작이나 안전 reset을 숨길 수 없다. 접근·manual native control·외부 소재 변경이 생기면 즉시 적절한 유형과 latched scope로 전환한다.

case scope는 command resource뿐 아니라 관련 위험 구역·잔류 native 작업·소재 지지를 포함한다. 영향이 불명확하면 셀 전체로 잡는다. 동시에 열린 다른 case의 scope와 겹치면 공통 조정 책임과 허용 행위를 검사한다. case 한 개 종료가 다른 case의 보호/차단을 해제하지 않는다.

## 2. 상태 전이

| 상태 | 진입에 필요한 기록/근거 | 허용/제한 |
|---|---|---|
| OPEN | 원인·영향 scope·책임 역할, 관련 작업/소재 | 관련 새 생산 보류. 조회·현지 보호 반응 가능 |
| CONTAINMENT_PENDING | 기존 permit/mandate 철회·fence 전파·정지/지지 요청/현지 반응 기록 | fence ack·명령 응답만으로 접근 허용 표시 금지 |
| PROCEDURE_ACTIVE | 현장 절차의 진입 조건을 충족했다는 **명시적 외부 근거**, 참여자·작업 범위 | 그 절차가 허용한 현장 작업만. 일반 생산 허가 없음 |
| REVALIDATING | 참여자 작업 종료/인계, 변경한 소재·장비·설정, 접근/격리 절차의 복원 단계 기록 | 현재 상태·잔류 명령·안전기능/모드·교정·영역·구성 다시 확인 |
| READY_FOR_RESTART | 필요한 재검토/처분 완료, 미결 모순 없음, 다른 case/외부 제한 해소 근거 | 새 시작 요청을 받을 준비. 자동 동작 없음 |
| CLOSED | 새 RestartRun 수용 또는 명시적 비운전 종료 처분 | 새 mandate의 범위만 실행 가능. 이전 mandate 부활 금지 |
| ESCALATED | 근거 모순/필요 기능·관측 부재/담당자 상실 | 관련 생산 차단 유지, 필요한 현지 보호·지원 절차 |

PROCEDURE_ACTIVE는 RX가 사람에게 접근 안전성을 인증했다는 상태가 아니다. 검증된 현장 절차/실제 보호·격리 기능이 제공하는 조건을 기록하는 상태다. 일반 operator acknowledge·password·원격 영상 확인만으로 대체하지 않는다. person/accountability/source가 없으면 UNKNOWN으로 유지한다.

DIAGNOSTIC_ONLY는 사람이 접근하지 않았고 **원 invocation과 검증된 정상 동작으로 설명되지 않는 물리 변화**, 보호 반응, 세대 상실이 없다는 근거와 원 invocation 결과를 회수하면 OPEN→REVALIDATING→CLOSED로 종료할 수 있다. 정상 집기/이동의 상관된 효과 자체는 이 경로를 막지 않는다. 이때만 02의 TRANSIENT 해제 후 원래 ACTIVE mandate를 계속 사용할 수 있다. 나머지 경로는 READY_FOR_RESTART와 새로운 RestartRun을 거친다.

어느 상태에서든 새로운 상실·사람/물체 변경·증거 충돌이 발견되면 이전 clearance를 무효화하고 CONTAINMENT_PENDING 또는 ESCALATED로 되돌아간다. 단계 번호 증가만으로 이후 조건을 자동 만족시키지 않는다.

## 3. 사람·접근·인수인계 기록

ProcedureRecord는 절차 ID/revision, 담당 actor, 행위 종류, 실제 발생/확인 시각, cell/scope, 물리 장비/인원/소재 참조, evidence source와 값, 해당 단계의 검증 결과를 갖는다. 행위는 `ACKNOWLEDGE`, `ENTRY_CONDITIONS_REPORTED`, `WORK_STARTED`, `WORK_FINISHED`, `PERSONNEL_ACCOUNTED`, `ISOLATION_STATE_REPORTED`, `RESET_OBSERVED`, `HANDOVER_ACCEPTED`, `CONFIGURATION_REPORTED`로 구별한다.

- ACKNOWLEDGE는 알림을 읽었음을 기록할 뿐 조건·outcome·접근·reset·시작을 변경하지 않는다.
- RESET_OBSERVED는 실제 안전기능/장비가 보고한 reset 또는 허용된 절차로 확인한 사실이다. 이 기록 API가 native reset을 실행하지 않는다.
- PERSONNEL_ACCOUNTED는 절차가 정한 참여자·구역 확인이다. 빈 명단·로그아웃·응답 없음은 사람 없음의 증거가 아니다.
- ISOLATION_STATE_REPORTED는 실제 장치·절차의 격리/복원 사실을 참조한다. RX 논리 lock을 물리 LOTO로 표현하지 않는다.
- 담당자 교체는 나가는/들어오는 역할과 남은 작업·인원·격리·소재 상태의 인수를 기록한다. 인수 완료 전 case는 진행 허가를 확대하지 않는다.

서로 다른 작업조/보전요원/운영자가 같은 scope에 있으면 개별 작업/인원 상태와 전체 조정 책임을 모두 유지한다. 한 사람이 자신의 작업을 끝냈다는 기록으로 다른 사람/외부 lock의 제한을 해제하지 않는다. 협력사 절차가 따로 있으면 그 참조·연속성을 포함한다. 구체 인원 수·격리 조작은 현장 절차의 입력이며 임의로 정하지 않는다.

## 4. 복구 계획

RecoveryPlan은 case·envelope·procedure revision·현재 material/장비 상태와 명시적인 허용 step의 유한 집합에 결합한다. step은 `OBSERVE / RECORD_EXTERNAL_ACTION / GUARDED_OPERATION / VERIFY / DISPOSITION` 중 하나다. GUARDED_OPERATION은 기존 operation 종류와 새 RECOVERY permit를 사용한다.

필수 항목: 각 step의 precondition·관련 scope·native 의미·완료 evidence·실패/중단 반응·다음 step·허용 actor. 생산의 precondition이 거짓이라는 이유로 recovery에서 모두 생략하지 않는다. 필요한 대체 guard와 실제 보호 조건을 명시해야 하며 일반 bypass flag는 없다.

`(case_id, plan_digest, step_id, visit)`는 P의 영속 유일 실행 slot이다. guarded step의 operation/digest와 같은 T1에서 연결하며 다른 request key로 같은 slot을 제출해도 같은 operation만 회수한다. 같은 slot의 다른 의도는 충돌이다. 다음 visit는 plan의 허용 edge/횟수·현재 case 상태·이전 step 근거를 P가 확인한 경우에만 새로 생성한다. 단순히 caller가 visit 숫자를 올리는 것으로 재실행할 수 없다. 유효 plan/guard 변경은 영향 scope를 invalidate해 옛 미진입 permit를 봉인하고 진입 작업을 조정한 후 적용한다.

native resume의 의미는 `CONTINUE_CORRELATED / RESTART_FROM_ENTRY / NO_RESUME`로 선언한다. CONTINUE_CORRELATED에는 이전 native invocation과 continuation token/지점, 앞서 수행한 효과, 현재 자세/소재·잔류 명령 근거가 있어야 한다. RESTART_FROM_ENTRY이면 그 프로그램이 앞부분을 다시 수행할 수 있으므로 원 작업의 자동 재전송으로 쓰지 않는다. 필요한 경우 현재 상태에서 출발하는 별도 검증된 복구 프로그램·새 operation으로 정의한다. NO_RESUME은 결과 조사/현장 절차로만 조정한다.

UNRESOLVED를 성공으로 바꾸지 않는다. 후속 생산을 재개하려면 과거 불명에 대한 처분과 현재 상태의 확인을 모두 기록한다. 되돌릴 수 없는 가공/소재 변경은 DB rollback처럼 취급하지 않는다. 대상 부품의 후속 사용/검사/보류 처분은 정의된 품질·현장 역할의 명시적 결과를 요구한다.

## 5. SR01 — 소재 공급 중 불명과 사람 개입

가정: 한 operation이 SEND_ENTERED이고 결과가 없어 소재 지지가 불명이다. 실제 설비에서 이 상황을 만든 시험이 아니다.

1. P는 기존 operation을 UNKNOWN/RECONCILING으로 유지하고 관련 후속 전달을 보류한다. 원 key로 조회하며 새 생산 호출을 하지 않는다.
2. 조회만으로 근거를 회수하면 DIAGNOSTIC_ONLY 경로를 검토한다. 사람이 필요하거나 연속성을 확인할 수 없으면 FAULT_RECOVERY case와 latched scope를 생성하고 mandate를 철회한다.
3. H가 fence를 설치해도 물리 정지/지지는 별도로 확인한다. 접근이 필요한 경우 외부 절차·기능이 정한 조건/참여자 근거 전에는 PROCEDURE_ACTIVE에 들어가지 않는다.
4. 작업자의 소재 재배치·수동 이동·tool/교정 변경을 기록하고 관련 MaterialState/Condition/Qualification을 무효화한다.
5. 현재 상태·인원/영역·native queue·mode/owner·필요 보호 기능·구성을 재확인한다. 불명 operation은 회수된 근거로 끝내거나 UNRESOLVED 처분한다.
6. case가 READY_FOR_RESTART이고 관련 다른 제한이 해소돼야 새로운 RestartRun을 받아 새 mandate와 명시된 시작/continuation 지점을 만든다.

명시적 재시작을 누르기 전에 준비가 완료돼야 한다. 재시작 클릭이 나머지 체크를 모두 true로 만드는 동작은 아니다. 재시작 후에도 H의 최종 gate와 실제 보호 조건은 계속 필요하다.

## 6. SR02 — 통신·전원·에너지 상실

상실 원인마다 scope·현지 반응·유지해야 할 상태·그 근거를 profile에서 정의한다. 로봇 접속 끊김, 관측만 끊김, 제어 source 상실, PLC/controller restart, 압력/전원/배터리 이상을 하나의 timeout 사유로 합치지 않는다.

고정식은 축/tool/소재의 유지, 바퀴형은 차체 위치/제동/도킹과 팔 동작, 능동 균형형은 전신/지지·반력·회복 공간을 포함한다. zero velocity·brake release·Damping·torque disable 등 native 이름만으로 안전 상태를 선언하지 않는다. 장비별 에너지 유지/격리 절차는 기능 할당·시험으로 결정한다.

P 또는 저장이 없어도 필요한 현지 반응은 동작해야 한다. 복원 시 이미 일어난 반응·기록 공백·장비 세대·현재 지지·잔류 명령을 조사한다. 권한/장비/물리 연속성이 깨졌거나 현지 보호 반응이 발생한 상실에서는 일반 생산 permit/mandate를 재사용하지 않는다. 연속성이 입증된 단순 관측/응답 보류는 02의 TRANSIENT 경로를 따르되 소비/만료된 permit는 언제나 재사용 금지다. runtime process 정상 종료 역시 기능/지지 조건을 따라야 하며 healthcheck failure가 driver 강제 재기동/torque 전환의 충분조건이 아니다.

## 7. SR03 — 구성·정책·기구 변경

ChangeRecord는 변경 전/후 artifact hash, 변경 이유, 실제 적용 여부, 영향 qualification/condition/recovery/기능/시험, 검토 역할과 disposition을 기록한다.

| 변경 | 반드시 검토할 영향 |
|---|---|
| tool·지그·교정·소재/하중 | 경로·착좌·지지·간섭·위험 범위·정지/접촉 조건 |
| robot/PLC firmware·controller·native program | 접수/결과/재시작 의미·잔류 queue·mode·신호 |
| policy/모델·controller gain·명령 schema | 출력/행동·범위·정지/지지·센서·authority·검증 조건 |
| layout/지도/도킹·바닥·구역 | 위치/접근·위험 영향 범위·동시동작·복구 공간 |
| 통신·Host·OS·CPU/GPU variant | freshness·지연·deadman·native ownership·공통 원인 고장 |
| 담당자/운영 절차 | 작업/인수·접근·상주가 필요한 조건·개입/재시작 |

설계 초안 변경만으로 현장을 바꿨다고 처리하지 않는다. 실제 적용은 `PROPOSED→IMPACT_REVIEWED→STAGED→APPLIED_UNQUALIFIED→QUALIFIED_ACTIVE`로 구별한다. STAGED의 새 artifact는 실행 선택 대상이 아니다. 영향 guard/시험을 확인하기 전에는 이전 qualification을 새 구성에 붙이지 않는다.

변경 적용에는 영향 scope의 새 epoch, 옛 permit 봉인, 진입 native 작업/지지의 처리, 관련 Host의 fence/구성 ack가 필요하다. 일부 Host만 바뀌었으면 MIXED_CONFIGURATION으로 격리한다. 알려지지 않은 의존 관계는 셀 전체 영향으로 처리한다. ‘품번이 같다’거나 ‘checksum 파일만 바뀜’은 영향 없음의 근거가 아니다.

## 8. 종료·포기·미완료

case CLOSED는 생산 재시작과 동일하지 않다. 비운전 종료는 `REMAIN_OUT_OF_SERVICE` 처분으로 closed 기록을 남기고 별도 latched block을 유지한다. 이를 해제하려면 새 개입/qualification 절차가 필요하다. unresolved physical scope를 단순 ‘작업 취소’로 잊지 않는다.

Clearance는 특정 case revision, cell/scope epoch, evidence와 target disposition에 결합한 1회 재시작 준비 기록이다. 어떤 변화가 생기면 무효다. 여러 case의 제한은 합집합으로 적용한다. 하나라도 남거나 알 수 없는 외부 격리/인원 상태가 있으면 관련 RestartRun을 거부한다.

비운전 종료에는 `PrepareClose`가 REMAIN_OUT_OF_SERVICE clearance를 발급한다. target run이나 restart plan은 요구하지 않는다. 대상 case의 작업/인원 인수·현재 위험 억제/격리 상태·남겨둘 제한의 근거를 확인하고 case revision과 같은 transaction에 묶는다. 진행 중 인원이나 미확인 외부 상태를 무시한 종료는 허용하지 않는다. CloseWithoutRestart는 이 clearance 소비·대상 case 종료·별도 out-of-service LATCHED block 생성을 원자적으로 기록하며 ArmCell을 호출하지 않는다.
