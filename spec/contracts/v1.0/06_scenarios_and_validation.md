# 반례 검토·검증 의무·요구 추적

범위: RX 계약 v1.0 · **문서 사건 추적 결과**. 실행 시험·실물 시험·TLC 모델 검사 결과가 아니다.

## 1. 검토 방법과 환경 가정

P=platform, H=Host, E=executor, D=device다. P/H는 임의 시점에 중단될 수 있고 메시지는 유실·중복·지연될 수 있다. native 명령이 시작된 뒤 중단된 process의 DB transaction이 장비 효과를 rollback하지 않는다. 정상 복원에서는 영속 journal이 보존된다고 가정하고, journal 유실/backup rollback은 별도로 다룬다.

각 trace에서 ‘P가 아는 것’과 ‘실제 D가 했을 수 있는 것’을 구별한다. 문서 판정은 제안 규칙이 해당 trace에서 어떤 상태·근거·차단을 요구하는지 검토한 결과다. 모든 가능한 실행 순서를 열거한 증명으로 표시하지 않는다.

## 2. 정상·유실·재시작 trace

| 사례 | 사건 순서와 주체별 지식 | 규범 결과·기각한 대안 |
|---|---|---|
| SC01 유한 trajectory | ① E가 activation A/slot S 제출 ② P가 O와 intent를 T1 저장 ③ H가 native goal G와 PREPARED 저장 ④ H SEND_ENTERED 후 G 전송 ⑤ D의 G 결과와 후조건 관측을 H가 저장 ⑥ P가 증거·SUCCEEDED·event를 T2 저장 | E는 P 결과를 조회해 분기. joint 성공의 의미만 가지며 grip/material 성공은 별도 증거. BT success만으로 끝내는 대안 기각. I01·04·07 |
| SC02 native 응답 유실 | ① H SEND_ENTERED ② D가 실행했을 수 있음 ③ 응답 경로 유실 ④ P는 timeout을 받지만 H/D 결과를 모름 ⑤ 같은 key 재제출은 O 반환 ⑥ H receipt를 조회하고 G 결과를 회수하거나 불명 유지 | RECONCILING/UNKNOWN, 자원 격리. 현재 idle만 있으면 성공도 미실행도 결정 불가. 같은 SDK 호출 자동 반복 기각. I02·03·04 |
| SC03-A P restart | ① P T1 저장/dispatch 대기 ② H PREPARED 또는 SEND_ENTERED ③ P만 재시작 ④ 새 P가 원장과 같은 O 복원 ⑤ H receipt/세대/미ack 증거 조회 ⑥ 권한 인계·현재 조건 검사 | PREPARED면 새 grant로 검토 후 진행 가능, SEND_ENTERED면 조회만. 원장 outbox를 무조건 재생해 생산하는 대안 기각. 과거 O 보존. I01·03·05 |
| SC03-B H restart | ① H가 SEND_ENTERED 저장 ② native 호출 직전 또는 직후 crash ③ P와 D는 살아 있음 ④ H가 journal 재개 ⑤ old grants/samples 폐기 ⑥ native 결과/현 상태 조회 | 호출 전 crash였더라도 SEND_ENTERED면 미실행이라고 단정하지 않음. 반복 대신 UNKNOWN. 가용성 일부를 포기해 자동 중복 native 호출을 막는다. I03·06 |
| SC03-C D/controller restart | ① O의 성공 이력 또는 진행 이력이 있음 ② D만 restart ③ H의 현재 sample/모드/native cache 연속성 상실 ④ 새 device session 경계 ⑤ firmware/mode/잔류 명령/물리 상태 재확인 ⑥ 새 readiness 평가 | 과거 영속 성공 기록은 보존, 현재 predicate는 무효화. 진행 O의 결과 cache가 사라지면 UNKNOWN. boot 관측이 없으면 ‘검출 보장 없음’을 profile에 남김. I04·09 |
| SC05 E restart | ① A/S→O가 P에 저장 ② D 완료·P 결과 commit ③ E가 응답을 못 받고 restart ④ E가 Run/Checkpoint/A/S 조회 ⑤ 기존 O를 회수 ⑥ 해당 결과로 다음 단계 제안 | 새 tick UUID를 새 production key로 쓰지 않음. 같은 node의 다음 반복은 새 visit의 activation이어야 한다. I02·11 |

## 3. 경합·관측·다른 작업 종류

| 사례 | 사건 순서와 주체별 지식 | 규범 결과·기각한 대안 |
|---|---|---|
| SC04 취소·완료 경합 | ① P cancel intent 저장 ② D는 이미 G 성공 또는 중단 중 ③ cancel 응답이 먼저 도착 ④ 뒤늦게 G 결과 도착 ⑤ profile와 correlation으로 양립성 평가 ⑥ quiescence/소재 인계 따로 확인 | cancel 접수만으로 CANCELED 아님. 성공 증거와 cancel 요청은 양립 가능; 모순되는 두 terminal native 결과면 DISPUTED. resource release는 별도. I04·08 |
| SC06 관측 단절 | ① 마지막 sample seq=120 GOOD ② 관측 경로만 끊김, 명령 경로는 살아 있음 ③ H가 캐시를 반복 조회 ④ 원본 seq/age는 그대로 ⑤ freshness 만료 ⑥ readiness와 후조건 평가 차단 | 캐시 함수 호출 시각으로 age를 갱신하지 않음. 신규 증거 없는 RUNNING/UNKNOWN 왕복은 결과가 아님. I04·09 |
| SC07 AS-02 mode | ① ReadyPose 요청 T1/H 기록 ② mode service 수락 ③ ModeStatus=ReadyPose 관측 ④ 실제 자세/안정 후조건은 아직 불충족 ⑤ 필요한 관측 대기 ⑥ 해당 rule 충족 또는 timeout/reconcile | 서비스 수락을 posture success로 매핑하는 대안 기각. mode-only rule과 자세 확보 rule은 다른 profile. I04·10 |
| SC08 FFW/leader stream | ① session/grant/ticket OPEN ② seq 증가 sample 수용 ③ source 단절 또는 owner 변경 ④ ticket/lease/deadman 만료 ⑤ 새 sample 차단+현지 expiry reaction ⑥ 잔류 명령/지지 확인 후 인계 | 느린 buffer sample을 새 권한으로 재생하지 않음. 0속도 명령 접수와 실제 정지 분리. 휴머노이드에는 동일 torque-off 대입 금지. I05·06·08 |
| SC09 DHI 기동·종료 | ① profile가 init/activate/destructor 효과 선언 ② 필요한 support/mode/calibration 확인 ③ lifecycle operation 기록 ④ native 초기화 또는 종료 진입 ⑤ shutdown 때 지지 인계 미확인 발견 ⑥ driver 정상 종료 보류 | healthcheck→activate 또는 shutdown timeout→torque-off 자동 승격 기각. 강제 power loss에 대한 별도 물리 보호는 후속 검증. I01·08·10·12 |
| SC10 패키지만 존재 | ① own package 모두 포함 ② 실제 controller/mode 또는 교정 mismatch ③ admission finding 생성 ④ native dispatch 없음 ⑤ 필요한 profile/교정 수정 ⑥ 새 검증 뒤 admission | 기본 포함을 지원 완료·실행 허가로 간주하지 않음. 해당 자사 모델의 구현 의무는 유지. I10 |
| SC14 arm/gripper 공유 JTC | ① A가 arm trajectory 요청, resource=controller/J ② B가 gripper 요청, 같은 J ③ T1에서 충돌 검출 ④ B BUSY ⑤ A 결과와 자원 해제 확인 ⑥ B를 새로 허용하거나 애초 하나의 compound trajectory로 설계 | 논리 arm/gripper 이름이 다르니 병렬이라는 대안 기각. compound trajectory는 하나의 검증된 native goal이어야 함. I05·08 |

## 4. protocol·저장 trace

| 사례 | 사건 순서와 주체별 지식 | 규범 결과·기각한 대안 |
|---|---|---|
| SC11 느린 subscriber | ① subscriber cursor=500 ② producer 계속 commit ③ buffer 한도 도달 ④ subscriber를 명시 종료 ⑤ 500부터 재구독, 중복 적용 방지 ⑥ retention 밖이면 동일 cut snapshot 후 through+1 | 제어 producer block/무한 buffer/사건 silent drop 기각. telemetry coalescing과 다름. I07·09 |
| SC12 혼합 schema/언어 | ① PeerHello hash 협상 ② 공통 hash 없으면 command 차단 ③ 공통 schema에서 필수·presence·unit 검사 ④ default 확장/JCS ⑤ P/H digest 비교 ⑥ mismatch면 native 호출 없음 | proto raw bytes나 언어 map iteration hash 기각. unknown enum을 0으로 바꿔 실행하는 대안 기각. I02·04·10 |
| SC13-A T1 저장 실패 | ① key/intent 검증 ② T1 commit 실패 또는 반환 유실 ③ 일반 dispatch 차단 ④ 같은 key 조회 ⑤ commit 확인되면 receipt 회수 ⑥ 확인 불가면 STORE_FAULT | 실패 응답을 받은 UI가 새 key로 재생산하는 대안 기각. I01·02·07 |
| SC13-B H journal 실패 | ① H PREPARED ② SEND_ENTERED commit 실패 ③ native 일반 호출 없음 ④ 보호 반응 필요하면 별도 경로 ⑤ 복원 후 journal 대조 | DB 저장 실패를 무시하고 ‘일단 움직이고 나중에 기록’ 기각. I01·12 |
| SC13-C D 완료 후 저장 실패 | ① SEND_ENTERED/실제 동작 ② 결과 저장 전에 H 또는 P write 실패 ③ P는 완료를 확정할 수 없음 ④ 현지 보호·증거 보존 시도 ⑤ 결과 회수 가능하면 T2 ⑥ 불가하면 UNKNOWN/UNRESOLVED | 실제 완료를 취소됐다고 기록하거나 자동 재실행하는 대안 기각. I03·04·07·12 |
| SC13-D 양쪽 이력 rollback | ① 과거 동작 후 오래된 백업 복원 ② 복원 절차가 generation 변경 ③ 옛 outbox 재발행 차단 ④ peer watermark 대조 ⑤ 현재 상태·소재·잔류 명령 조사 ⑥ commissioning/recovery disposition | 복원 사실을 숨긴 동시 rollback의 완전 검출은 보장 범위 밖. ‘백업이면 언제든 자동 재개’ 기각. I03·05·09 |

## 5. 추가 반례와 검토 중 보강한 규칙

| 반례 | 보강 |
|---|---|
| 재전송한 lease renew가 만료를 계속 늘림 | 증가 renew_seq, 중복은 원래 응답. prepare validity는 lease와 별도 고정 |
| 네트워크에 오래 있던 stream frame이 재연결 직후 최신처럼 적용 | Host-issued 만료 ticket, source boot/seq, latest-only sample. source 자체의 stale 데이터 검사도 필수 |
| 성공한 뒤 장비가 reboot했으니 과거 성공을 실패로 수정 | 과거 outcome와 현재 observation 유효성을 별도 관리 |
| CANCEL 후 종료 결과만 보고 중력을 받던 부품의 지지를 해제 | RELEASED에 물리 인계 조건 요구 |
| subscriber filtering으로 seq가 비는데 이를 packet loss로 오인 | v1은 고정된 권한 view 전체 stream. view별 cursor와 filter 부재를 명시 |
| DB volume을 Host와 공유해 둘이 같은 state를 수정 | 업무 원장은 P만 writer, Host는 별도 receipt journal |
| 여러 native call을 하나의 Adapter 함수 안에서 몰래 재시도 | RX composite는 자식 operation으로 분해. OEM native composite는 부분 결과 한계를 명시 |
| 새 role/auth token을 받은 뒤 옛 command key namespace가 바뀜 | peer/client identity와 credential 수명을 분리 |
| T2 ticket의 seq=11 뒤 유효한 T1 ticket의 seq=10 도착 | 같은 control session/source boot 전체 high-watermark로 seq=10 거부. ticket 갱신이 순번을 초기화하지 않음 |
| grant 검사 직후 thread 정지, 새 owner 인계 뒤 이전 thread 재개 | 검사·SEND_ENTERED·native 제출과 revoke를 같은 gate로 직렬화. 진입한 호출이 남아 있으면 인계 금지 |
| PREPARED 폐기 직후 늦은 Authorize가 작업을 되살림 | VOIDED_BEFORE_SEND와 SEND_ENTERED의 단일 CAS, P outbox도 NEW→VOIDED/EMIT_ENTERED 경쟁 |
| pause/다음 단계 이후 늦은 미사용 activation slot 도착 | T1에서 run/checkpoint/activation/권한 적격성을 원자 검사. 이미 있는 mapping 조회만 허용 |

## 6. 후속 형식 검증 범위

TLA+ 모델의 최소 변수는 `P_intents, P_outbox, P_results, H_receipts, H_evidence, native_calls, grants, fences, messages, crashes`다. 전이는 Admit, Prepare, Authorize, NativeCall, RecordEvidence, CommitResult, Cancel, Expire, CrashP/H/D, Deliver/Drop/Duplicate, Restore다.

검사할 safety 속성은 I01–I12다. 특히 `production_native_calls[operation] ≤ 1`은 **유한 작업/상태 쓰기/모드·기동 전이의 자동 retry 없는 production invocation** 범위에서 검사한다. 명시적 취소는 `cancel_native_calls[cancel_id] ≤ 1`, stream은 session/source_seq 증가, I12 현지 보호 명령은 별도 incident로 모델링한다. 하나의 native 호출 내부 하드웨어 반복/연속제어를 ‘1회 물리 변화’로 해석하지 않는다.

검사할 liveness는 ‘영속 저장·통신·관측·소유권이 회복되고 필요한 근거가 생기면 RECONCILING 작업은 결과/명시적 UNRESOLVED 처분에 도달할 수 있음’이다. 장비가 결과를 전혀 제공하지 않는 환경에서 자동 성공이나 무조건 진행을 증명 대상으로 삼지 않는다. TLC bounded model 검사와 실제 계약 구현의 정합성 시험 모두 후속 의무다.

## 7. 구현·현장 검증 의무

| 검증 ID | 할 일·필요 산출물 | 담당 역할 | 통과 전 제한 |
|---|---|---|---|
| V01 | Rust/C++ protobuf presence·unknown/duplicate field·enum·JCS/digest·schema hash golden fixture | 플랫폼+연동 | 양쪽 protocol 적합 주장 불가 |
| V02 | 각 T1/T2/T3/Host SEND_ENTERED 전후 crash·응답 유실·중복·disk-full 주입 | 플랫폼+검증 | 재시작 보장 출시 불가 |
| V03 | 단일 writer·fence·lease·renew replay·권한 인계·외부 native client 검출 | 연동+운영 | 자동 제어권 인계 불가 |
| V04 | 각 own support ID/모드별 실제 driver/firmware/교정/관측/중단/기동·종료 시험 | 자사 플랫폼 담당 | 해당 profile 생산 admission 불가 |
| V05 | SC14 공유 JTC와 bus/공간 자원 충돌 확인 | 로봇·공정 | 독립 병렬 제어 허가 불가 |
| V06 | PLC OEM I/O map·handshake·실제 feedback·boot·잔류 command·operator recovery 계약 | PLC/OEM+공정 | 문/척/가공 시작 binding 활성 불가 |
| V07 | sample age·deadman·ticket·현지 정지/지지·CPU/GPU 부하·장치 분리 시험 | 제어+검증 | 제어 stream 성능/반응 보장 불가 |
| V08 | 백업/복원·generation 변경·journal 유실·동기화·전원 상실 저장 시험 | 배포+운영 | 무인 자동 복원 불가 |
| V09 | snapshot 동시 쓰기·pagination 만료·7일 retention·느린 subscriber·store 압력 | 플랫폼+운영 | 장기 운영/진단 적합 주장 불가 |
| V10 | 불명/모순/개입/포기/현재 상태 재확인 UI와 audit 인수 | 제품+현장 운영 | 작업자 복구 절차 인수 불가 |
| V11 | TLA+ bounded safety/liveness 및 spec-to-implementation 대조 | 계약+검증 | 형식 검증 완료 주장 불가 |

‘담당 역할’은 조직의 실제 개인 배정 완료를 뜻하지 않는다. 시험 시간·cycle 수·threshold는 현장 profile 위험/요구에 맞춰 시험 명세에서 정해야 한다. 임의 숫자를 검증 완료 기준으로 꾸미지 않는다.

## 8. 요구 추적과 설계 완료 감사

| 요구/항목 | 주 문서 | 상태 |
|---|---|---|
| 경계·책임 C01–C08 | 01 §1, 03 §5, 04 | v1.0 설계 확정 |
| 작업 분류·의미 | 01 §2–7 | v1.0 설계 확정 |
| 상태·식별·복구 D01–05·08–09 | 01–02 | v1.0 설계 확정 |
| 순서·세대·재구독·시간·buffer D06 | 02 §5–7, 03 §7–8 | v1.0 설계 확정 |
| 데이터·전송·호환 D07·10 | 03 | v1.0 설계 확정 |
| 다른 모델·mode·version 반례 | 04 및 본 문서 SC01–14/A–D | 문서 trace 검토 수행; 실험 아님 |
| 대안·선택·제약·후속 검증 | 05, 조사 보고서, 본 문서 V01–11 | 근거·의무 작성 |

최종 독자 검토와 수정 내역은 `review_record.md`에 남긴다. V01–V11은 현재 실행하지 않았으며 코드 착수 지시 이후의 구현/납품 검증 단계에 남는다.
