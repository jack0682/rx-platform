# 반례와 검증 의무

상태: 2026-09-14 제조사 중립 문서 개정. 아래는 문서 사건 추적이며 제품 실행·형식 모델 검사·실물/안전기능 시험 결과가 아니다.

## 1. 확인할 불변식

| ID | 계약이 지켜야 하는 것 |
|---|---|
| OI01 | 허용할 operation의 envelope/qualification/의존 hash·현재 scope가 일치한다. |
| OI02 | 필요한 조건의 UNKNOWN을 PASS로 만들거나 미지 schema를 묵살하지 않는다. |
| OI03 | native 생산/설정/복구 쓰기는 해당 purpose/parent의 cell-aware gate를 통과한다. |
| OI04 | revoked mandate는 복원되지 않는다. 정상 대기의 transient 해제와 구별한다. |
| OI05 | pending StartAttempt/Arm ack/clearance는 자체로 native 실행·접근 허가가 아니다. |
| OI06 | operation permit는 ID/내용/Host/epoch/purpose에 결합하며 소비/만료 뒤 재사용하지 않는다. |
| OI07 | 같은 소재의 지지 감소 작업은 공통 자원으로 충돌 처리하며 현재 상호 지지 PASS만으로 동시에 해제하지 않는다. |
| OI08 | 관련 열린 case/인원/외부 제한을 빠뜨려 생산 재시작을 허용하지 않는다. |
| OI09 | 알림 확인·reset 관측·작업 종료·조건 재확인·새 시작을 별도 행위로 처리한다. |
| OI10 | 관련 변경이 이전 condition/qualification/clearance를 무효화하고 새 epoch 동기화를 요구한다. |
| OI11 | controller/Host/P/executor 재시작·backup rollback에서 옛 mandate/permit로 자동 동작하지 않는다. |
| OI12 | 지식상의 정합성·data GOOD·UI 표시를 실제 안전기능 성능으로 주장하지 않는다. |
| OI13 | control stream의 허용 세션/epoch/순번과 실제 native 적용 순서를 유지한다. |
| OI14 | DB/통신/일반 생산 조건 불능이 필요한 현지 보호 반응을 막지 않는다. |
| OI15 | part 시도·node activation·operation 예산을 구별하고 재시작으로 사용량을 환급하지 않는다. |
| OI16 | 지원 package 포함·문서 승인과 실제 commissioning을 구별한다. |
| OI17 | Host의 cell/scope epoch는 후퇴하지 않으며 오래된 Arm/Fence로 block을 지우지 않는다. |
| OI18 | P의 invalidation과 H 적용 사이 시간창을 숨기지 않고 필요한 물리 반응을 현지 기능에 할당한다. |

## 2. 정상·보류·재시작의 trace

기호 P=platform, H=Host, E=executor, D=장비, U=작업자다. 영속 기록과 현재 관측을 구별하고 source가 모르는 사실을 추론하지 않는다.

| 사례 | 사건 순서 | 계약에 따른 결론 |
|---|---|---|
| CO01 정상 자동 반복 | ① qualified envelope+새 StartRun ② H Arm ack ③ P mandate commit ④ part_attempt 1개 생성/예산1 소비 ⑤ 그 part의 6개 node 각각 T1/permit/결과 ⑥ 다음 part 생성 | node 6개가 소재 예산6을 소비하지 않는다. 각 native는 별도 결과/조건, 정상 run 내부에 추가 human click 없음. OI01·03·15 |
| CO02 응답 유실·정상 효과 회수 | ① 집기 O/G native 정상 효과 ② 응답만 유실 ③ P TRANSIENT·동일 key 조회 ④ 같은 G 결과와 설명되는 물체 이동/지지 확인 ⑤ 무개입·무보호반응·동일 세대 연속성 확인 ⑥ transient 해제 후 다음 node | 원 native 재호출 없음. 실제 정상 이동이 있었다고 diagnostic 경로를 무조건 금지하지 않음. 기존 ACTIVE mandate 유지 가능. OI04·06 |
| CO03 불명·사람 개입(SR01) | ① O SEND_ENTERED ② 조회로 지지/결과 미확인 ③ FAULT_RECOVERY와 latch/fence/철회 ④ 현장 진입 절차의 외부 근거 ⑤ 수동 조치 기록·재검증 ⑥ UNRESOLVED 처분 또는 결과 회수 | old success 조작 없음. 준비된 epoch/clearance와 새로운 RestartRun만 생산을 허용. OI04·08·09 |
| CO04 reset 단독 | ① 안전정지로 mandate 철회 ② D reset 보고 ③ U 알림 확인 ④ 현재 조건 일부 PASS ⑤ StartIntent 없음 | 새 mandate/permit 없음. reset 또는 popup 닫힘을 start로 매핑하지 않음. OI05·09 |
| CO05 재시작 자기 무효화 방지 | ① cases REVALIDATING/e7 ② PrepareRestart가 e8 fence ③ 잔류 명령·상태 재확인 ④ READY 전이와 전이 후 case revision/e8 clearance를 같은 commit ⑤ U RestartRun·동일 e8 Arm ⑥ P clearance소비+case종료+새mandate | clearance(e7)를 만들고 e8로 바꾸거나 READY 전이로 자기 case revision이 stale가 되는 순서를 금지. OI05·10·17 |
| CO06 부분 Arm | ① H1 Arm ack ② H2 응답 없음 ③ P attempt ARMING ④ old request 재전송 또는 timeout | 새 mandate commit/생산 permit 없음. H1 gate 준비 자체로 생산할 수 없음. OI03·05 |
| CO07 정상 WAIT vs 보호 반응 | ① 정상 자재 대기 FAIL/UNKNOWN ② 조건복원(변경·개입 없음) / 또는 ②′ 현지 보호반응 발생 ③ 분류 재검사 | 첫 경로만 transient 해제. ②′는 latch/철회 후 새 시작. 단어 ‘대기’만으로 분류하지 않음. OI02·04 |

## 3. 경합·사람·소재·변경 trace

| 사례 | 사건 순서 | 계약에 따른 결론 |
|---|---|---|
| CO08 지연 invalidation | ① P가 permit 발행 ② P가 조건 상실을 알고 latch/fence 저장 ③ 통신 지연 중 old permit H 도착 ④ H local guard 또는 실제 보호 경로 반응 ⑤ fence 설치 | P DB 시각부터 즉각 물리 차단됐다고 주장하지 않음. fence 이후 old epoch 진입 금지; 필요한 반응 시간은 local 기능의 의무. OI17·18 |
| CO09 옛 epoch replay | ① H A8/B6 설치 ② 지연 A7/B6 Fence 또는 old Arm 도착 ③ local map 검사 | STALE 거부, map snapshot replace 금지, 새 block 삭제 없음. OI17 |
| CO10 여러 case/작업자 | ① 같은 scope에 C1/U1·C2/U2 존재 ② U1 작업 종료·C1 준비 ③ C1만 넣은 RestartRun ④ C2/외부제한 검사 | C2가 남으면 거부. 인수 기록·빈 명단만으로 U2가 떠났다고 하지 않음. OI08·09 |
| CO11 recovery motion 필요 | ① 생산 guard 불충족·case 진행 ② plan에 명시된 상태확인/제어 operation 필요 ③ purpose RECOVERY+case/plan/step+대체guard 평가 ④ H gate | 생산 guard를 일괄 skip하지 않음. 허용된 recovery step 외 motion 불가. 필요한 현지 보호는 계속 유효. OI03·14 |
| CO12 tool 변경과 시작 경합(SR03) | ① clearance/tool vA로 준비 ② vB 실제 적용 보고와 StartAttempt 경합 ③ 같은 cell/case revision CAS ④ 영향 closure/epoch/qualification 갱신 | 옛 target으로 mandate 최종 commit 불가. 일부 Host만 변경되면 MIXED_CONFIGURATION. OI01·10 |
| CO13 양쪽 지지 해제 | ① G와 C 각각 support PASS ② G해제와 C해제가 서로 다른 Host에 제출 ③ P가 같은 material/support resource CAS ④ 한 요청만 예약 ⑤ 결과/현재 지지 확인 후 다음 판단 | 두 native가 서로의 옛 PASS를 보고 동시 해제하지 않음. 실제 지지 유지의 별도 기구/기능 검증 필요. OI07 |
| CO14 공유 JTC | ① arm과 gripper가 논리적으로 다른 operation ② 같은 controller resource ③ permit 발급 예약 경합 | 직렬화 또는 검증된 하나의 native composite trajectory. 이름이 다르다는 이유의 병렬 허용 없음. base SC14+OI03 |
| CO15 scope 독립 | ① RunA는 scope A, RunB는 검증된 독립 B ② A만 latched ③ A closure 계산·Aepoch 증가 ④ B vector/조건 유지 | 검증된 독립 B는 영향 없는 경우 계속 가능. mapping 불명은 셀 전체 차단. OI01·10·17 |
| CO16 clearance 잘못된 재사용 | ① e8·runA·planA·cases{C1,C2} 준비 ② runB 또는 planB 또는 C2 누락으로 요청 ③ target/revision/body 비교 | 거부. 허가가 비슷한 작업의 일반 token으로 쓰이지 않음. OI05·06·08 |

## 4. 장비 차이·상실·버전 trace

| 사례 | 사건 순서 | 계약에 따른 결론 |
|---|---|---|
| CO17 driver 기동 반환 | ① guarded lifecycle 시작 ② source의 torque enable 내부 시도 실패/미확인 가능 ③ callback SUCCESS ④ 실제 torque/지지 evidence 부재 | 관련 조건 UNKNOWN, 생산 준비로 승격하지 않음. source의 성공 문자열을 완료 evidence로 과장하지 않음. OI02·16 |
| CO18 정책 기반 제어 모드 | ① software startup 또는 ReadyPose/policy 요청 ② native command publishing 가능 ③ mode/status와 실제 자세/지지 따로 관측 | 출력 경계가 cell gate 밖이면 해당 binding 부적합. 필요한 package가 설치되어도 검증 전 자동 활성하지 않음. OI03·12·13 |
| CO19 바퀴형/족형(SR02) | ① 작업 중 base 위치 또는 전신/에너지 조건 상실 ② 필요한 local response ③ P/RPC가 없을 수 있음 ④ 현재 위치/지지/잔류 명령 재조정 | zero velocity/Damping/torque-off 이름만으로 정지·지지 보장 없음. 모델/모드/하중·환경별 기능 할당/검증 요구. OI12·14 |
| CO20 안전 사본 GOOD | ① Mirror packet은 새 GOOD ② 실제 보호 기능이 해제/고장일 가능성 ③ 전체 기능 근거·현재 signal scope 검사 | 데이터 quality를 safety function 성능으로 채택하지 않음. 필요한 근거가 없으면 qualification/조건 불충족. OI02·12 |
| CO21 저장 실패 | ① P/H 저장 불능 ② 새 일반 permit/production 전달 차단 ③ 현지 보호 필요 ④ 가능한 근거 보존/복원 | protection은 DB commit을 기다리지 않음. 기록 실패가 이전 물리 효과를 취소하지 않음. OI06·14 |
| CO22 백업/재부팅 | ① P backup에 e5, H는 e9 ② P복원 후 H Inspect/원장 대조 ③ max보다 새 epoch/fence ④ 상태/qualification/case 재검토 | e6을 일방적으로 강요하거나 옛 permit 재생하지 않음. peer 이력을 못 읽으면 준비 완료 아님. OI11·17 |
| CO23 구버전 우회 | ① cell-enforced 배포 ② base-only UI/Host가 StartRun/Authorize 직접 호출 ③ mandatory feature/endpoint 정책 검사 | UPGRADE_REQUIRED/권한 오류, fallback native 동작 없음. old journal에 cell event를 silent skip하지 않음. OI03·16 |
| CO24 run 재시작 예산 | ① run 예산10, part 시도4 소비 ② 불명/개입 후 새 mandate ③ 동일 run의 남은 예산 조회 ④ 같은 part continuation 또는 신규 part5 | 사용량4 보존. 같은 part의 node는 추가 part 소비 없음. unknown를 환급해 무제한 생산하지 않음. OI15 |
| CO25 정상 종료·driver 소멸 | ① shutdown 요청 ② driver destructor가 torque disable 가능 ③ 소재/중력 지지 아직 필요 ④ lifecycle 조건 검사 | 종료/컨테이너 재시작 정책이 기구 지지 조건을 우회하지 않음. 강제 전원 상실은 별도 physical protection 범위. OI03·14·16 |
| CO26 비운전 종료 | ① 재시작 계획 없이 운전 포기 ② PrepareClose로 작업/인원/제한 근거 확인 ③ REMAIN_OUT_OF_SERVICE clearance ④ close와 별도 latch 생성 atomic | 가짜 run/plan 요구 없음. Arm 호출/생산 허용 없음. 다른 사례 제한 유지. OI05·08 |
| CO27 recovery 재시도 key 변경 | ① case/plan/step/visit1이 O에 연결 ② 같은 slot을 새 key로 제출 ③ 영속 unique slot 조회 | 같은 의도면 O 회수, 다른 의도면 충돌. visit2는 plan edge·근거/상태/한도 검사. OI03·06 |
| CO28 오래된 UI의 실제 변화 보고 | ① clearance 준비 뒤 실제 수동 조작/접근 발생 ② 옛 case revision으로 보고 ③ 신뢰된 사실/evidence와 latch 먼저 기록 ④ 단계 CAS 충돌 반환 | 오래된 revision이라는 이유로 실제 변화가 버려지지 않음. 새 허용 전이는 없고 재시작 준비 무효. OI08·09·10 |

## 5. 원문·사실과 검증 범위

CO17·18과 CO14의 원래 조사 출처는 [고정 원문](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/cell_operations/v1.0/05_validation_audit.md)에 보존한다. 현재 표는 해당 실패 유형을 제조사 중립적인 구성에 적용한 문서상 반례다. 새 driver source나 실물을 검증한 결과가 아니다. 나머지는 계약의 비동기/사람/변경 모델에 대입한 가상 사건 trace다. ‘trace에서 규칙상 어떤 결과여야 하는가’를 검토했으며 실제 장비가 그렇게 동작했다고 하지 않는다.

정적 문서 검토에서 남기는 증거는 조건/전이/API/역할이 위 결론을 일관되게 요구하는지다. 실제 구현이 그 요구를 만족하는지는 아래 검증에서 확인해야 한다. 형식 모델 검사의 변수 후보는 cell/scope epoch, block sets, mandates, permits, cases, clearances, budgets, messages, native-entry count와 source validity다. Safety/liveness라는 형식 검증 용어를 기계 기능안전 인증과 혼동하지 않는다.

## 6. 후속 구현·현장 검증 의무

| 검증 | 필요한 증거·판정 | 적용 범위/담당 |
|---|---|---|
| OV01 조건 evaluator | UNKNOWN/FAIL/ANY·ALL·모순·빈 조건·unit/schema/age의 golden fixture | platform·Host 계약 구현 |
| OV02 permit/fence 경합 | 발급·소비·철회·Arm·epoch rollback·lost ack 각 commit 경계의 fault test, native 진입 추적 | platform/Host, base V01–03 확장 |
| OV03 재시작 | PrepareRestart target epoch·clearance·case revision·all-host barrier·late event·복수 case trace | platform·운영 |
| OV04 native 복구 의미 | controller별 continue/entry/없음, 이미 수행된 물리 효과·현재 지지·잔류 명령 | OEM·로봇/장비 연동 |
| OV05 소재 지지 | 지지/착좌/파지의 감지 한계·상실 거동, 양쪽 해제 충돌·support resource와 실제 보호 | 기구·로봇·PLC·설치 |
| OV06 안전기능 | H01–H06에서 필요한 실제 sensor/logic/output/drive 경로·반응 시간·요구 성능/달성 근거·고장/환경/하중 | 제작팀·OEM·기능안전/현장 검증 |
| OV07 사람 개입 | 접근/격리·인원·인수/외부 제한·reset/start·현장 UI의 사용자 검증 | 현장 운영·설치 |
| OV08 변경/배포 | partial update·새 policy/tool·source/Host change·backup rollback, mandatory cell capability 우회 거부 | 릴리스·플랫폼·솔루션 |
| OV09 선택한 장비 구성 | 선택한 profile별 실제 mode·driver/stream·기동/종료와 외부 직접 native 쓰기 경계 | 각 장비 연동 담당 |
| OV10 예산/실적 | PartAttempt vs activation/operation, 동일 key, 재시작 잔량, unknown/불량/취소 이력 | platform·제품 |
| OV11 형식·적합성 | bounded 모델의 OI01–18, 구현의 대응 trace, schema/manifest와 생성 결과 적합성 | 계약·검증 |

이 표는 검증 의무다. 실제 이행 여부는 해당 구현 commit과 실행 근거를 따로 대조하며 이번 문서 개정에서 재실행하지 않았다. 실제 반복 횟수·판정 수치·시간을 입력 없이 채우지 않는다. 미충족된 현장 조합은 NOT_COMMISSIONED/REVALIDATION_REQUIRED로 차단한다. 공통 소프트웨어 설계의 문서 확정과 특정 납품의 기능/현장 검증을 구분한다.

## 7. 초판 감사와 현재 검토의 구분

N01–N06의 초판 완료 감사·독립 검토 결과는 [2026-09-10 당시 원문](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/cell_operations/v1.0/05_validation_audit.md)에 보존한다. 초판 감사에서 사용한 모델·지원 의무·source inventory를 현재 중립 구성의 검증 결과로 바꾸지 않는다.

현재 개정은 장비 지원 정책과 적용 예시를 변경했고 공통 의미·허가·복구의 불변식과 검증 의무를 유지한다. [개정 기록](revision_2026-09-14.md)에 영향·반례·hash·호환 범위를 남겼다. 실제 모델·신호·기구·인원·보호 성능·시간 수치는 아직 확인되지 않은 profile의 입력으로 남기며 추측해 채우지 않는다.
