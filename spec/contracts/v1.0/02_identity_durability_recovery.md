# 식별·영속성·제어권·복구

규범: RX 계약 v1.0 · [의미 계약](01_responsibility_and_semantics.md)

## 1. 식별과 동일성

| ID | 발급·범위 | 재사용 규칙 |
|---|---|---|
| installation_id | 설치 등록, UUID | 장치 이름·IP와 별개. DB 복원 때도 원래 이력 소유 표시 |
| store_generation | 복원/초기화 절차, UUID | 정상 프로세스 재시작에는 유지. 백업 복원·이력 유실에는 변경하고 재조정 |
| run_id / activation_id / operation_id | platform, UUIDv7 | 각각 회차·단계 방문·작업의 영속 ID. 시각으로 인과 순서를 판단하지 않음 |
| request_key | UI/업무 client, UUID; BT는 영속 activation+slot | scope=(installation, client namespace, API method, key). 연결·인증 token 갱신에도 같은 client namespace 유지 |
| invocation_id | Host가 PREPARED commit 때 UUID 발급 | 같은 operation에는 한 번 발급. ROS goal UUID 등 native ID와 연결 보존 |
| boot_id | 각 프로세스 시작마다 UUID | 프로세스 세대 구분. 실제 장비 boot ID와 혼동 금지 |
| journal_id + seq | 각 영속 journal | seq는 1부터 단조 증가. process 재시작에는 계속, journal 교체는 ID 변경 |
| resource_fence | platform이 자원별 durable 증가 정수 발급 | Host가 받아 저장한 최대값보다 작은 값 거부. v1은 하나의 Runtime만 실행 |

같은 request_key와 같은 정규 의도는 저장된 operation을 반환한다. 다른 의도면 `KEY_CONFLICT`다. 같은 내용이라도 새 key면 새 생산 작업일 수 있으므로 hash만으로 합치지 않는다. 동시 요청은 DB unique constraint/transaction으로 한 개만 생성한다.

Submit의 key 일치 검사에는 intent_digest뿐 아니라 `(run_id, activation_id, slot)` binding tuple도 포함한다. 같은 key를 다른 단계에 재사용하면 내용이 같아도 KEY_CONFLICT다. 일반 mutation의 key fingerprint는 method와 업무 body에서 session/call/key/expected_revision 및 서버 발급 ID를 제외한 정규 값이다. credential/연결 변경과 CAS 재시도는 업무 의도를 바꾸지 않는다. Cancel은 원 operation/digest/cancel_rule, Recovery는 원 operation/절차/근거/처분을 반드시 포함한다. 감사 reason의 변경도 별도 요청으로 구별한다.

Host의 **delivery journal**(PREPARED/SEND_ENTERED 등)과 **evidence outbox journal**은 서로 다른 journal_id·seq 공간이다. receipt가 delivery seq=20이고 첫 evidence가 outbox seq=1이어도 정상이다. P는 receipt를 (delivery journal ID, seq)로 dedup하고 evidence batch를 (evidence outbox ID, seq)로 dedup한다. receipt 조회값도 판단에 사용하기 전에 P의 T2에 불변 근거로 기록한다.

v1은 operation ID·scope/key·의도 digest·마지막 결과/무결성·기록 위치의 **작은 tombstone을 설치 이력 수명 동안 보존**한다. 큰 관측·진단 데이터에는 별도 retention을 적용할 수 있다. 원장을 완전히 폐기한 설치는 새 installation으로 commissioning하며 옛 key를 새 작업으로 수용하지 않는다. 보존이 불가능하면 신규 admission을 중단한다. ‘오래된 key이니 처음 본 요청’으로 되돌리지 않는다.

## 2. platform commit 경계

단일 로컬 SQLite DB의 transaction port를 기본 저장 binding으로 선택한다. durable commit에는 WAL + synchronous FULL, 지원 로컬 파일시스템과 검증된 저장장치가 필요하다. 두 컨테이너가 같은 DB 파일을 직접 열지 않는다. DB 파일만 복사하는 백업은 금지하고 일관성 있는 backup 절차를 사용한다. SQLite 구현은 Rust domain core 밖의 저장 adapter다.

| transaction | 같은 commit에 포함할 레코드 | commit 뒤 허용 |
|---|---|---|
| T1 admission | key→operation, 정규 의도·digest, activation/slot 연결, resource reservation, OperationAdmitted 사건, dispatch outbox | Host Prepare 요청 |
| T2 evidence | Host inbox dedup cursor, 불변 evidence, operation revision/판정, 사건, 대응 outbox ack | Host에 durable ack, UI 사건 노출 |
| T3 workflow | 기존 checkpoint revision 검사, activation/slot별 자식 할당, 새 checkpoint, 사건·outbox | executor에게 checkpoint 수락 반환 |
| T4 cancel/authority | cancel intent 또는 fence/권한 변경, 사건, Host 전달 대기 | native cancel/권한 재조정 요청 |
| T5 recovery disposition | 조사 근거·actor·이전 결과 참조, 자원 해제 조건, 새 admission 조건, 사건 | 새 run/activation 허용 |

outbox는 NEW와 EMIT_ENTERED를 durable 구별한다. sender는 네트워크 호출 전에 NEW→EMIT_ENTERED를 CAS하며, 미발행 취소는 같은 행을 NEW→VOIDED로 CAS한다. VOIDED이면 sender가 발행할 수 없다. EMIT_ENTERED 이후에는 Host에 아직 안 보이더라도 지연 메시지가 있을 수 있으므로 NOT_FOUND만으로 NOT_EXECUTED를 만들지 않는다.

outbox의 전달 ack는 물리 완료가 아니다. commit 반환이 유실돼 commit 여부가 불명하면 같은 key를 조회한다. 조회로 확인하기 전 새 key로 재요청하지 않는다. 로컬 DB가 자체적으로 손상돼 조회가 불가하면 STORE_FAULT 상태를 유지한다.

executor/run에 속한 operation의 T1은 같은 transaction에서 run=EXECUTING, 현재 executor 권한 세대, activation의 실행 적격성, 허용 slot·visit, recipe/schema/profile revision을 검사한다. Pause/Abandon/단계 완료와 새 slot admission은 같은 run revision/CAS 경계에서 직렬화한다. 이미 존재하는 key의 조회·결과 회수는 run이 멈춘 뒤에도 허용하지만, 늦게 온 미사용 slot을 새 operation으로 만들지 않는다. pause는 이미 EMIT_ENTERED인 native 작업을 취소했다는 뜻이 아니므로 별도 cancel/recovery를 수행한다. run 없는 단독 operation은 site configuration에서 허용한 setup/manual 역할과 mode에서만 받으며 동일한 profile·자원·현재 조건 검사를 거친다. UI가 run 검사를 피하기 위해 생산 요청을 단독 작업으로 바꾸지 못하도록 API 권한을 구분한다.

executor 권한 세대는 RunView에 연결된 현재 executor Session ID다. StartRun은 site configuration에 지정된 peer의 현재 authenticated session을 run에 결합한다. T1의 CallContext.session_id가 그 값과 일치해야 한다. E restart/disconnect로 새 session이 생기면 이전 연결의 늦은 요청을 거부하고 run을 PAUSED/RECOVERY_REQUIRED로 둔다. checkpoint 회수·미결 operation 조정 뒤 명시 StartRun으로 새 session에 결합한다. 새 session 생성만으로 기존 run을 자동 재개하지 않는다.

## 3. Host journal과 단일 native 전달

Host는 solutions 전용 영속 volume에 inbox/전달/증거 outbox를 저장한다. 이는 업무 상태의 두 번째 권위 원장이 아니라 **platform이 직접 알 수 없는 native 전달 사실의 기록**이다.

1. `PrepareOperation`을 받으면 동일성·profile·세대·자원·권한을 검사한다. `operation_id, digest, invocation_id, state=PREPARED`를 durable 저장한다.
2. `AuthorizeDispatch`를 받으면 Host의 현지 유효 grant·기동 세대·freshness·interlock·device session을 전달 직전 다시 검사한다.
3. 같은 invocation을 `SEND_ENTERED`로 durable commit한다. 이 전이는 process-wide 직렬화와 journal CAS로 단 한 주체만 수행한다.
4. commit에 성공한 주체만 native 요청을 최대 한 번 호출한다. SDK 내부 재시도도 포함해 v1 profile은 native effect의 자동 반복을 금지한다. 읽기 조회 재시도와 byte-level 전송 재시도의 물리 의미를 구별해 검증해야 한다.
5. native 응답·결과를 evidence outbox에 durable 기록하고 platform에 발행한다. platform durable ack 전에는 삭제하지 않는다.

**검사와 호출 사이의 경합**: Host의 command gate는 grant/fence 검사, SEND_ENTERED CAS, native 제출 진입을 취소·grant 철회·인계와 직렬화한다. gate 안에서 검사만 한 뒤 lock을 풀고 나중에 native 호출하는 구현은 금지한다. 이미 진입한 native 호출이 return하지 않거나 thread가 정지하면 새 owner에게 해당 resource를 넘기지 않는다. 현지 보호 반응은 별도 경로로 수행하되 그것만으로 pending 호출이 사라졌다고 간주하지 않는다.

Host가 PREPARED를 취소하면 같은 gate/CAS로 **VOIDED_BEFORE_SEND** tombstone을 durable 기록한다. 지연 Prepare/Authorize는 이 tombstone을 조회하고 거부한다. 아직 inbox에 없는 operation의 명시적 취소도 operation_id+digest를 가진 VOIDED_BEFORE_SEND를 생성할 수 있도록 cancel request에 digest를 포함한다. tombstone과 SEND_ENTERED는 둘 중 하나만 먼저 성립한다. SEND_ENTERED가 먼저면 미실행으로 취소할 수 없고 native 결과/중단을 조정한다.

PREPARED에서 복구하면 native 호출 전임을 안다. SEND_ENTERED에서 죽었다면 실제 호출 전이었을 수도, 장비 실행 후였을 수도 있다. **그 구간을 UNKNOWN으로 남기는 것이 선택한 보장**이다. 자동으로 다시 호출하지 않는다. 데이터베이스 기록과 물리 동작 사이의 원자성을 보장한다고 주장하지 않는다.

`AuthorizeDispatch`의 중복 호출은 현재 receipt를 반환한다. 재접속 후 PREPARED 작업을 진행하려면 새 제어 grant와 현 상태 검사를 거친다. 상태가 SEND_ENTERED 이상이면 조회/조정만 한다. native가 중복 제거를 지원해도 v1의 불명 재전송 금지는 유지한다. 추후 특정 binding의 명시적 확장으로만 완화한다.

취소는 T4가 발급한 별도 `cancel_id`를 target operation/digest/cancel_rule과 연결한다. Host는 cancel_id별 PREPARED→SEND_ENTERED→결과 전달 기록을 갖고 같은 취소를 재수신해 native cancel을 반복 호출하지 않는다. 원래 production receipt와 cancel receipt는 별개다. 새 cancel_id로 다시 정지를 요청하려면 현재 권한과 profile의 반복 정지 허용 조건을 새로 검사한다. 취소 응답 유실이 새 cancel_id를 자동 생성하는 이유가 되지 않는다. 제어 sample에는 sample별 fsync를 적용하지 않고 §6의 session/seq 규칙을, 현지 보호 반응에는 §8의 저장 독립 예외를 적용한다.

## 4. 제어권과 자원

제어권은 사용자 권한, workflow 실행권, 장비 명령권을 구별한다. UI 로그인만으로 로봇 명령권이 생기지 않는다.

- Runtime은 단일 writer와 OS 프로세스 배타 잠금으로 두 active 인스턴스를 막는다. 둘 이상의 컴퓨터에서 같은 저장소를 복제해 자동 active로 만들지 않는다.
- Host는 실제 command endpoint를 소유한다. 동일 resource에 대해 최대 fence와 유효 grant를 journal에 저장하고 이전 fence 요청을 거부한다.
- 새 owner를 승인하기 전에 이전 owner의 대기 요청을 무효화하고 현지 stream 종료·native 잔류 명령·소재 지지를 확인한다. ack를 못 받으면 자원은 QUARANTINED다. lease 만료만으로 새 owner에게 넘기지 않는다.
- 이미 OS/network/device queue에 들어간 이전 command도 인계 대상이다. RX gate 철회만으로 native queue가 비워졌다고 판정하지 않는다. 이전 native channel/세션의 종료·대기 호출의 종료·장비의 잔류 명령 배제를 profile의 근거로 확인해야 한다. 이를 확인할 방법이 없으면 자동 인계를 허용하지 않는다.
- native API를 직접 호출하는 다른 client나 pendant는 RX token을 검사하지 않을 수 있다. profile은 native mode/remote authority 검출 또는 OEM 배타 연동 방법을 지정해야 한다. 이것이 없으면 RX만의 배타성이라는 제한을 표시하고 허용 범위를 줄인다.
- 충돌 단위는 논리 `arm/gripper` 이름이 아니라 controller/버스/모드/물리 작업 공간 등 profile의 `resource_set`이다. 동일 JTC로 팔과 gripper를 보내면 같은 명령 자원이다.

v1은 작업 시작 때 필요한 모든 자원을 한 transaction에서 예약하고 부분 획득 대기를 금지한다. 자원이 없으면 BUSY와 재요청 힌트를 반환한다. 자원 subset을 나중에 추가 획득하는 방식은 deadlock 회피 규칙을 추가하기 전 허용하지 않는다. 충돌 없는 병렬 작업만 허용한다.

묶음 resource_set의 새 fence는 `max(각 자원의 저장된 fence)+1`로 계산해 한 transaction에서 모두에 저장한다. Host의 equal fence는 기존 동일 owner·grant 요청의 재생에만 허용한다. 새 owner·grant에 equal fence를 재사용할 수 없다. 자원 인계의 실제 완료 전 새 fence를 기록한 경우 자원은 격리 상태를 유지한다.

## 5. 시간과 grant

시간 제한은 용도별로 분리한다. RPC deadline은 응답 대기, admission validity는 아직 전달하지 않은 의도의 유효 기간, execution timeout은 관측/복구 전환, observation age는 근거 유효성, lease는 명령권, deadman은 stream 입력 유효성이다.

Host가 grant를 발급할 때 `grant_id, host_boot_id, resource_fence, receiver_expiry`를 보유한다. expiry는 Host monotonic clock으로만 판정한다. peer의 wall clock deadline을 그대로 비교하지 않는다. 요청자는 opaque grant와 순번을 제출한다. 갱신은 살아 있는 authenticated session에서 증가하는 renew_seq만 수용하며 중복 갱신은 만료 시각을 연장하지 않는다. 만료 후 갱신은 거부하고 새 권한 획득 절차를 밟는다.

grant를 무한 갱신해 오래 대기한 동작을 나중에 실행하지 않도록 operation의 `prepare_validity_ms`를 PREPARED 시점 Host monotonic 기준으로 별도 고정한다. Host restart면 그 validity는 복원하지 않고 PREPARED를 재검토 대기로 둔다. wall clock 변경·시계 동기화 실패가 명령 수명을 늘리지 않는다.

장비 실행 timeout은 profile의 값이며 임의의 전역 5초로 통일하지 않는다. 재시작 후 이전 monotonic 시간축을 복원할 수 없으면 진행 시간은 불명으로 기록하고 새 생산 동작 없이 재조정한다. 기록용 UTC는 사건 표시용이며 다른 주체 간 인과 순서를 결정하지 않는다.

v1의 같은 Linux 호스트 배포는 P/H가 **같은 host boot identity와 CLOCK_BOOTTIME 시간축**을 검증해 공유하는 것을 필수 조건으로 한다. 서로 다른 time namespace offset은 허용하지 않는다. `clock_id=<host-boot-uuid>/boottime`로 식별하며 application process boot_id와 다르다. 이 조건을 확인하지 못한 연결은 현재 상태 freshness를 필요한 범위로 검증할 수 없으므로 관련 작업을 admission하지 않는다. 미래의 다른 호스트/OS 배포에는 검증된 clock 변환·불확실성 bound의 별도 transport profile이 필요하다.

P가 현재 조건을 판정하는 시점의 age 상한은 `P.now - H.receive_time + acquisition_uncertainty`다. 동일 clock_id, GOOD quality, profile가 허용한 freshness_basis, 유한한 acquisition_uncertainty가 있어야 한다. 따라서 H outbox에서 오래 대기한 관측은 P 도착 시점에 fresh가 되지 않는다. 원본 시각을 같은 시간축에 매핑할 수 있으면 더 엄격한 source age 상한도 검사한다. 캐시의 원본 시각/seq를 모르면 CACHED_UNKNOWN이며 현재 조건의 유효 근거로 사용할 수 없다. 과거 invocation의 immutable native 완료 이력은 현재 predicate와 달라서 오래됐다는 이유만으로 지워지지 않는다.

## 6. 제어 스트림

CONTROL_SESSION을 OPEN하려면 source·command schema·mode·resource·deadman·expiry reaction을 먼저 고정한다. 팔로워/leader/정책 출력을 생성하는 process와 Host는 solutions 내부에서 연결된다. platform RPC로 제어 sample을 실시간 relay하지 않는다.

각 sample은 `session_id, source_boot_id, source_seq, host_ticket, values`를 가진다. Host가 발행한 짧은 유효 ticket에는 Host monotonic 만료가 결합된다. 지연되어 뒤늦게 도착한 ticket의 sample은 거부한다. 같은 ticket의 여러 sample은 증가하는 seq만 수용하며 ticket은 원래 만료를 연장하지 않는다. sample cache는 resource당 최신 유효 값 하나이고 이전 값 재생·disk queue는 금지한다.

sample의 session_id는 인증 Session ID와 구별되는 **CONTROL_SESSION operation_id 자체**다. H는 이 ID를 grant/resource/source_boot/schema/mode에 결합한다. 마지막 수용 source_seq는 (control operation_id, source_boot_id) 전체에 걸쳐 단조 증가하며 ticket 교체로 초기화하지 않는다. 두 ticket이 겹쳐도 이미 받은 seq보다 작거나 같은 sample은 재적용하지 않는다. source_boot가 바뀌면 기존 세션을 닫고 새 operation을 생성해야 한다.

sample의 실제 native 적용도 자원별 **단일 consumer/command gate**로 직렬화한다. 수용 handler가 각자 native 쓰기를 실행해서는 안 된다. consumer는 최신 cache를 고르고 gate 안에서 session/grant/ticket 유효성과 마지막 적용 seq를 검사한 뒤 native 제출까지 수행한다. 검사와 제출 사이에 다른 sample 적용·세션 종료·권한 인계가 끼어들지 못한다. 적용 seq는 단조 증가하며, 제출에 진입한 호출이 정지/불명이면 새 소유자에게 자원을 넘기지 않는 §3 규칙을 따른다. seq10 handler 지연 후 seq11 적용 뒤 seq10이 재개되는 순서 역전을 금지한다.

source가 오래된 관측을 새 sample로 포장하는 문제는 ticket만으로 해결되지 않는다. source의 관측 age 검사·freshness provenance도 stream profile의 필수 조건이다. source/Host 재시작·mode 변경·grant 만료·deadman 발생 시 세션은 닫히고 새 sample 수용을 막는다. 현지 expiry reaction을 실행한 뒤 그 결과를 관측한다.

세션 종료 이유는 REQUESTED, EXPIRED, SOURCE_LOST, MODE_CHANGED, DEVICE_RESET, STORE_FAULT 등으로 기록한다. 종료 요청 처리와 실제 정지/지지 확인을 구별한다. `0 velocity` 전송도 모델별 반응 명령이며 휴머노이드에 일반적인 torque-off를 강제하지 않는다.

## 7. 재시작·연결 복원

| 사건 | 폐기/무효화 | 복원·확인 후 가능한 일 |
|---|---|---|
| platform만 restart | 이전 Runtime session의 미발행 권한, 오래된 UI readiness | DB 조회, Host receipt/증거 회수, fence 재조정. 기록된 operation ID 유지. 자동 run 재개는 하지 않음 |
| Host만 restart | 모든 volatile grant·control session·sample, 현지 monotonic validity | journal에서 PREPARED와 SEND_ENTERED 구별, 장비 현재 상태·모드·잔류 명령 확인 |
| ROS controller/PLC/device restart | 현재 observation·mode·native result cache·control session의 유효성 | native boot 근거와 profile 재조회. 완료 이력은 보존하되 현재 상태 predicate는 새로 확인 |
| 장비 restart 감지 수단 없음 | 부팅 연속성을 보장할 수 없음 | 의심/연결 갱신 때 새 device-session 경계 설정, 필수 상태 재확인. 조용한 reboot까지 검출했다고 주장하지 않음 |
| BT executor restart | 메모리 tick 상태·로컬 pending promise | platform checkpoint와 activation-slot mapping 조회. 기존 operation 결과를 사용하고 새 ID로 재발행하지 않음 |
| DB 백업 복원·journal 유실 | 자동 실행·이전 lease·옛 outbox 전달 | store_generation 변경, Host와 마지막 watermark 비교, 모든 관련 자원 격리, 현장 재조정 |

백업이 되돌아간 사실 자체를 숨긴 관리자 복제/볼륨 rollback까지 애플리케이션만으로 탐지한다는 보장은 없다. 운영 복원 절차가 store_generation을 바꾸고 Host와 교차 확인하는 것이 지원 조건이다. 이력 양쪽을 동시에 잃었으면 이전 작업 결과는 UNRESOLVED이며 새 commissioning이 필요하다.

## 8. 저장 실패와 정상 종료

platform DB write가 실패하면 새로운 생산 admission/dispatch를 차단한다. Host가 증거를 보존할 수 있으면 outbox에 유지한다. Host journal 실패도 신규 native 효과를 막는다. 이미 수행된 물리 동작이 저장 실패로 취소됐다고 기록하지 않는다.

미리 준비한 **현지 보호 반응**은 저장 불능·네트워크 단절 때도 동작해야 한다. 정지/hold/전이의 구체 내용은 profile과 장비 제어가 정한다. 이 경로는 journal commit을 기다리지 않는 I12의 명시적 예외다. 가능한 증거를 보존하고 연결 복원 후 결과 불명/기록 공백을 알린다. 소프트웨어 보호 반응은 인증된 안전 기능 자체가 아니다.

정상 종료 순서는 `신규 작업 차단 → stream/작업 정리 요청 → 잔류 명령·소재 지지 확인 → torque 해제 가능 확인 → driver 종료 → journal flush`다. DHI destructor가 torque disable을 수행하므로 torque 유지가 필요한 상태에서 driver를 정상 종료하지 않는다. graceful shutdown timeout이 끝났다고 강제 종료를 자동 승인하지 않는다. 강제 kill·전원 상실의 물리 결과는 별도 장비 설계·검증 대상이다.
