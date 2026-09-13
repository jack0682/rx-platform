# 사용 범위·운전 조건·위험과 기능

규범: RX 셀 운영 계약 v1.0 · 원문 선택 근거는 [조사 보고서](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/cell_operation_research_2026-09-10/research_and_decisions.md)의 S01–S13/O-D01–11을 따른다.

## 1. 책임과 셀 경계

`Cell`은 위험·제어·소재 인계가 결합된 운영 범위다. 장비 1대, 컨테이너 1개, 제조사 1개와 같다고 정의하지 않는다. 설치 엔지니어가 `CellDefinition`에 장비/command resource/위험 구역/소재 위치/Host/외부 보호 기능과 의존 관계를 등록한다. platform은 이 정의를 검증하고 판정·기록에 사용한다.

| 역할 | 소유할 정보·행위 | 경계 |
|---|---|---|
| 제작팀·OEM | 장비의 실제 기능·사용 한계·native 명령과 결과·정지/지지·진입 조건·검증자료 | 해당 제품/버전의 보장. 셀 전체 검증과 구별 |
| 설치·공정 엔지니어 | 기구·공정·접근·결합 위험·제어 범위·복구 절차·통합 검증 | 공급사 자료에 없는 의미를 signal 이름만으로 추정하지 않음 |
| 검증·릴리스 역할 | 평가·시험 범위와 구성의 일치, 미결·제약·사용 범위 검토 | 문서 서명 자체가 실물 상태의 근거는 아님 |
| 현장 운영·개입 역할 | 시작 의도, 정의된 현장 절차의 수행/확인·인원/소재/변경 기록 | 일반 알림 확인과 안전 reset·접근 절차를 구별 |
| platform | 조건 판정·의도·permit·무효화·개입·복구·이력의 권위 | 실시간 안전기능 성능을 DB 상태로 대체하지 않음 |
| Host | 전달 직전 조건·epoch·permit 검사, native 연동·관측·현지 반응 | 결과 판정 원장과 물리 보호 기능의 책임을 혼합하지 않음 |

## 2. 설치별 사용 범위 기록

이 절은 특정 고객·제품을 공통 계약의 전제로 삼지 않는다. 실제 설치마다 다음 입력을 확정한다. 기존 모의 셀과 과거 산업 적용 기록은 [보존 자료](https://github.com/jack0682/rx_docs/blob/main/references/README.md)에 있으며, 새로운 현장 검증을 대신하지 않는다.

| 항목 | 정의할 범위 | 실행 허용에 필요한 입력 |
|---|---|---|
| 용도 | 운반·인계·시설 서비스·제조 등 해당 설치의 작업 | 실제 시연/운영/교시 범위와 운용 절차 |
| 시설 제어기 | PLC·출입·승강기·충전 등 연결 대상 | 프로그램·통신 설정·신호 의미·고장/재부팅 동작 |
| 장비와 물품 | 로봇·tool·지그·적재 및 지지 구성 | 설치 개체·모델·버전·교정·좌표·하중 조합 |
| 작업 | 접수·실행·인계·최종 완료 조건 | 실제 순서/병행 범위와 각 참여자의 책임 |
| 사람 | 개입·접근·수동 조작·수령의 역할 | 개입 목적/위치·정비·인수인계·재시작 절차 |
| 검증 | 해당 구성의 물리 기능과 사용 한계 | 환경별 검증 결과·미결·제약 |

필수 입력과 근거가 확정되지 않은 설치는 `NOT_COMMISSIONED`다. 이는 현장 입력의 상태이며 소프트웨어 설계 결과의 실패가 아니다. 알 수 없는 값을 default true나 ‘해당 없음’으로 채우지 않는다. 설비의 기동 책임을 RX로 자동 배정하지 않는다. [프로젝트 범위](https://github.com/jack0682/rx_docs/blob/main/docs/03_product_scope.md)를 함께 따른다.

## 3. 검증한 사용 범위

`OperatingEnvelope`는 사용 가능한 조합을 정의하는 불변 artifact다. 필수 구성은 다음과 같다.

- cell/장비 instance·controller/firmware/driver·계약 및 profile 버전.
- recipe와 step 의미, 허용 operation kind/purpose, native continuation 방식.
- tool·지그·교정·소재 품종/형상·하중·접촉/지지 조건·위치/영역.
- 허용 운전/제어 모드, 동시동작, 사람 접근/개입/정비 범위, 필요한 운영 인원 조건.
- 바닥·경사·공간·조도 등 실제로 의존하는 환경과 에너지/압력/배터리 조건.
- 조건 정의·위험 행·안전기능 명세·복구 절차·검증 결과와 그 적용 범위.

개별 최대값의 곱집합을 자동 허용하지 않는다. 예를 들어 최대 하중 시험과 최대 reach 시험이 따로 있어도 두 조건을 동시에 만족하는 조합이 검증됐다고 하지 않는다. 허용 조합은 명시적인 variant/범위 제약으로 표현하고, 범위 밖 및 해석 불가는 거부한다.

`Qualification`은 envelope와 그 의존 artifact hash에 대해 검증 근거·검토 역할·제약·철회 정보를 기록한다. 상태는 `DRAFT / IN_REVIEW / QUALIFIED / SUSPENDED / RETIRED`다. QUALIFIED 전이는 필수 근거 연결과 검토 기록을 요구한다. 미충족 항목이 있으면 그 항목과 관련 없는 명시적 축소 envelope를 새로 정의·검증할 수 있지만 원래 요구 범위를 통과로 표시하지 않는다. 미충족 profile은 검증 범위와 미결을 기록하며 이미 승인한 사용 범위를 소급 확대하지 않는다.

## 4. 조건의 의미

`ConditionDefinition`은 ID·revision·적용 purpose/step·원본 source·평가 규칙·단위·유효성·상실 영향을 고정한다. runtime 결과는 `PASS / FAIL / UNKNOWN`이고 이유와 evidence를 함께 기록한다.

| 조건 종류 | 평가 시점 | 상실의 의미 |
|---|---|---|
| ADMISSION | operation을 새로 전달할 때 | 다음 전달 보류. 이미 시작한 작업의 유지 조건과는 별개 |
| MAINTAINED | 특정 동작/모드가 유지되는 동안 | 지정된 현지 반응·전달 제한·mandate 철회/개입 정책 적용 |
| WAIT_TARGET | 정상 recipe가 명시적으로 대기하는 목표 | 아직 도달하지 않음은 정상 대기일 수 있음. 고장 조건/기한은 별도 |

조건 평가 규칙은 버전 고정된 제한 grammar의 `ALL, ANY, EQ, RANGE, SET_CONTAINS`다. 임의 script·네트워크 조회·새로운 native 쓰기를 평가 함수에 넣지 않는다. EQ는 같은 typed value만 비교하고 부동소수 tolerance는 명시적 RANGE로 표현한다. RANGE는 단위·하한/상한·각 경계 포함 여부를 필수로 갖는다. 의미 없는 unit 변환/반올림으로 pass를 만들지 않는다.

ALL: 하나라도 FAIL이면 FAIL, FAIL 없이 UNKNOWN이 있으면 UNKNOWN, 모두 PASS이면 PASS. ANY: 하나라도 PASS이면 PASS, 모두 FAIL이면 FAIL, 나머지는 UNKNOWN이다. 필수 조건 집합은 항상 ALL이다. ANY는 검증된 대체 근거 경로에만 사용할 수 있고, 근거가 충돌하면 우선 UNKNOWN/incident로 처리한다. unknown input에 부정을 적용해 true를 만드는 규칙은 제공하지 않는다. 필요한 반대값은 `EQ(false)`처럼 원본 존재·유효성을 먼저 검사한다.

Observation의 source/boot/seq/quality/age는 공통 v1.0 규칙을 따른다. schema/unit/구성/세대 불일치, 원본 시점 불명, 모순, 필요한 신호 누락은 UNKNOWN이다. FAIL과 UNKNOWN 사유 모두 보존한다. cached GOOD를 새 sample로 꾸미지 않는다.

## 5. 신호·현재 조건과 실제 보호 기능

source 용도는 `PROCESS_STATE`, `SAFETY_STATUS_MIRROR`, `FUNCTION_PATH_SIGNAL`, `HUMAN_RECORD`로 구별한다. 선언만으로 safety grade가 생기는 것은 아니다.

- PROCESS_STATE는 공정 상태의 근거다. ‘문 닫힘’이 잠금·진입 안전·레이저 기동 허가까지 포함하는지 별도다.
- SAFETY_STATUS_MIRROR는 실제 기능의 표시/기록용 사본이다. 그 사본의 GOOD를 실제 기능이 항상 동작한다는 증명으로 쓰지 않는다.
- FUNCTION_PATH_SIGNAL은 해당 안전기능 경로에서 쓰이는 신호다. 센서·논리·통신·출력·구동의 전체 할당과 요구 성능·고장 대응·검증 범위가 있어야 한다.
- HUMAN_RECORD는 사람의 행위/확인 근거다. sensor self-test나 정지 성능을 대신하지 않는다.

위험 감소가 필요한 시간 내 반응을 일반 RPC/DB 원장만으로 달성한다고 가정하지 않는다. 어떤 구성요소가 기능 경로에 포함되는지와 지연·전원·공통 원인 고장·자원 경합을 명세해야 한다. 미검증 기능은 해당 위험에 의존하는 생산/접근 조건을 만족시키지 못한다.

## 6. 영향 범위와 소재

CellDefinition은 `condition→operation/step`, `resource→관련 resource/zone`, `artifact→qualification/condition/recovery`의 의존 관계를 갖는다. 영향은 연결 관계의 closure로 계산한다. 닫힌 범위 밖의 병행 실행을 허용하려면 물리 위험과 제어 독립성의 검증 근거가 있어야 한다. 정의 누락/모순으로 범위를 알 수 없으면 해당 셀 전체 새 생산을 차단한다.

`MaterialState`는 물체/slot 식별 근거, 현재 위치 주장, 여러 지지 주체와 각 evidence, 마지막 변경/개입 세대를 기록한다. 식별 근거는 sensor/추적 slot/사람 선언을 구별하며 실제 물체 identity가 검증되지 않으면 그 한계를 남긴다. 그리퍼+척 공동 지지는 허용하며, 단일 owner lock으로 물리 지지를 대신하지 않는다.

지지 주장의 PASS는 해당 소재·tool·하중·자세·진단·freshness 범위에서만 유효하다. 수동 이동·압력/전원 이상·교정 변경·소재 교체·관련 신호 모순으로 무효화한다. sender 해제 조건은 receiver 지지의 필요한 근거와 실패 대응을 포함하며 구체적인 해제 순서는 현장 기구/공정 명세가 정한다.

지지를 감소시키는 operation은 controller 외에 **동일 소재/인계 세대의 support resource**를 공통 resource_set에 포함한다. P는 서로 다른 Host의 그리퍼 해제와 척 해제가 동시에 예약되지 않도록 기존 T1의 전역 자원 CAS를 적용한다. receiver의 현재 PASS만으로 상호 해제를 허용하지 않는다. 해제의 전 구간에 필요한 다른 지지가 유지되는지 profile·현지 보호 경로/기구로 검증하고, 결과·새 지지 상태를 확인하기 전 support resource를 해제하지 않는다. 물체 alias/slot이 같은 실제 소재를 가리키는지 불명하면 지지 감소를 허용하지 않는다.

## 7. 첫 셀의 위험·기능·조건 연결 카드

아래는 확인된 작업 설명에서 도출한 위험 시나리오 카드다. 실제 위험등급/PLr/합격값은 부여하지 않았다. 각 `OPEN_*`은 commissioning 입력 요구이며 실행 default가 아니다.

| 카드 | 위험 경로·필요한 제약 | 할당/근거 요구 | 완료 전 제한 |
|---|---|---|---|
| H01 진입/레이저 | 접근 가능한 상태에서 방사·열·가공 동작이 남음 | OEM의 진입 가능 상태·방호·작동 억제 기능, 설치팀의 결합 접근 평가; OPEN_LASER_SCOPE | 진입·가공 연동 허용 금지 |
| H02 자동문 | 문 사이의 사람/팔/tool/소재가 문 동작과 충돌 | OEM 문 기능·감지/방호/정지, 설치팀 접근/영역 검증; OPEN_DOOR_FUNCTION | 문 자동 작업 허용 금지 |
| H03 소재 인계 | receiver 고정 미확인 상태의 sender 해제로 낙하/끼임 | 기구·로봇·OEM의 지지/착좌/고정 의미, 압력/전원 이상 거동; OPEN_SUPPORT | 해당 인계/해제 허용 금지 |
| H04 이탈/가공 | 팔/tool/소재 잔류 중 가공 시작 | 이탈 영역·관측 범위·OEM 가공 허용, 설치팀 간섭 평가; OPEN_CLEARANCE | 가공 start 연동 허용 금지 |
| H05 개입/재기동 | 사람/정비 중 이전 command 또는 전원/통신 복구로 동작 | 현장 접근/격리/인원 확인, native queue·reset/start 의미; OPEN_INTERVENTION | 생산 재개·접근 안내 허용 금지 |
| H06 상실/안정성 | 정지·무전원 때 축/소재/몸체 지지 상실 | 모델별 현지 반응·지지·에너지 명세/시험; OPEN_LOSS_RESPONSE | 해당 모델의 물리 작업 허용 금지 |

`SafetyFunctionSpec`은 function ID와 위험/사용 범위, trigger, 센서/판단/통신/출력/구동의 할당, 목표 상태·유지 조건, 요구 반응시간/성능 및 출처, 고장 가정·검출·대응, reset/재시작, 검증 방법·결과·제약을 필수로 가진다. 숫자나 근거가 없으면 해당 spec은 QUALIFIED가 아니다. “안전 PLC 사용”이라는 장치 이름만으로 모든 필드를 충족시키지 않는다.

위험 감소를 제어 안전기능에 할당한 항목에 SafetyFunctionSpec을 요구한다. 구조로 위험을 제거/감소시킨 항목은 그 설계·검증·잔존 위험의 근거를 연결하고 제어 기능의 비적용 사유를 남긴다. 모든 위험에 임의로 PLr을 부여하거나 모든 ROS package에 안전 인증을 요구하는 모델이 아니다. 사용정보·교육만으로 충분하다고 자동 판정하지 않고 채택한 위험 감소 조치의 적합성을 검토한다.

고정식 모델은 축/tool/소재 지지, 바퀴형은 위치·도킹·제동/주행과 팔의 결합, 능동 균형형은 전신 지지·반력·정지/회복 공간·에너지 조건을 추가한다. 같은 stop/Damping/zero velocity 명칭이 같은 물리 상태를 뜻하지 않는다. 각 지원 ID는 [장비 지원 정책](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md)의 model/mode 단위로 연결한다.
