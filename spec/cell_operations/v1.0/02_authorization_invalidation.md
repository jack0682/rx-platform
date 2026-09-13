# 시작 의도·작업 허가·무효화

규범: RX 셀 운영 계약 v1.0 · P=platform, H=Host. 공통 v1.0의 single-writer·key·epoch·journal·native 전송 gate를 재사용한다.

## 1. 독립된 상태

CellContext는 `revision, cell_epoch, scope_epochs, active_envelope, qualification, desired_mode, acknowledged_hosts, block_reasons, open_cases`를 가진다. desired_mode는 `SETUP / AUTOMATIC / RECOVERY / MAINTENANCE`이며 실제 native mode의 관측과 구별한다. mode 이름 변경만으로 native 움직임을 시작하지 않는다.

cell_epoch는 셀 definition/활성 구성/Runtime 세대의 전체 경계다. scope_epochs는 CellDefinition에 등록된 영향 구역별 세대 map이다. 국소 invalidation은 영향 closure의 scope epoch만 올리고, 셀 구성 교체·Runtime 복원처럼 전체 의미가 바뀌면 cell_epoch와 모든 scope를 갱신한다. 한 operation/mandate는 자신이 의존하는 전체 scope vector에 결합한다. 의존 관계가 입증된 다른 run/scope는 국소 사건만으로 자동 중단하지 않는다.

qualification이 QUALIFIED여도 block_reasons가 있을 수 있고, Run이 EXECUTING이어도 다음 operation이 대기할 수 있다. `READY`는 UI의 현재 파생 표시일 뿐 저장해서 영구 재사용할 권한이 아니다. block 해제와 RunMandate 복원도 별개다.

## 2. RunMandate

RunMandate는 **특정 run/recipe/envelope, 목적·수량 또는 유한 작업 범위, cell_epoch와 scope vector, 발급 actor와 시작 행위**에 결합한 영속 시작 의도다. 상태는 `ACTIVE / REVOKED / EXHAUSTED / CLOSED`이며 한번 REVOKED 된 ID를 ACTIVE로 되돌리지 않는다.

최초 생성 조건: 유효한 qualification, cell/recipe 구성 일치, 해당 mode/role, 관련 열린 개입 사례 없음, 필수 시작 조건 PASS, 현재 executor session, 명시적인 시작 요청을 검사한다. RestartRun은 지정된 READY_FOR_RESTART 사례/clearance만 새 mandate commit과 원자적으로 소비·종료할 수 있다. 다른 열린 case나 외부 제한은 예외가 아니며 재시작을 막는다. 조건 확인·Host 준비·mandate commit·native dispatch를 구별한다.

재시작 준비는 `PrepareRestart`에서 먼저 새 scope epoch를 설치해 옛 생산·복구 permit를 봉인하고, 진입한 native 작업/잔류 queue/지지 및 현재 조건을 재확인한다. **그 새 epoch에서 case의 READY_FOR_RESTART 전이와 clearance 발급을 같은 transaction으로 기록**하고, clearance에는 전이 후 case revision 집합을 넣는다. 이 상태에서는 새 recovery operation도 수용하지 않는다. 추가 작업이 필요하면 clearance를 폐기하고 REVALIDATING/PROCEDURE_ACTIVE로 돌아간다.

시작/재시작은 StartAttempt로 접수한 뒤 관련 Host의 `ArmCell` ack를 수집한다. RestartRun은 PrepareRestart가 만든 동일 epoch/clearance를 사용하며 여기서 다시 epoch를 올리지 않는다. ArmCell은 RX gate의 명시된 block만 해제할 준비를 기록하며 물리 reset/출력을 실행하지 않는다. 모든 Host가 일치하는 epoch로 준비되고 attempt의 cell/case revision이 유효할 때에만 P가 새 mandate·run 시작·clearance 소비·대상 case 종료를 같은 transaction으로 commit한다. 준비 중 invalidation이 오면 attempt를 거부하고 보호/차단을 유지한다. 관련 없는 case의 block을 함께 지우지 않는다.

생산 시작 출처는 `OPERATOR` 또는 envelope가 명시적으로 허용한 `BATCH_SCHEDULER`다. 스케줄러 권한은 정해진 주문·수량·recipe 범위만 포함한다. 사람 개입/안전정지/복구 후 `RestartRun`에는 복구 책임이 있는 OPERATOR의 새로운 시작 의도가 필요하며 scheduler가 이를 대행할 수 없다. 최초 시작과 복구 재시작을 별도 API/permission으로 구별한다.

예산 단위는 `PART_ATTEMPT / OPERATION_COUNT`로 명시한다. 소재 생산 run은 PART_ATTEMPT를 사용하며 P가 유일 part_attempt_id를 처음 영속 생성할 때 1회 소비한다. 그 소재 시도에 속한 집기·공급·문 등 여러 activation은 같은 소비에 속한다. SETUP run은 OPERATION_COUNT로 새 operation의 T1 생성 때 1회 소비한다. 같은 key/ID 조회는 어느 경우에도 추가 소비하지 않는다.

run 예산 원장은 mandate와 별도로 영속 보존한다. 재시작해 새 mandate를 만들어도 기존 소비량을 0으로 만들지 않는다. 결과 불명·불량·미전송 취소도 시도 이력을 삭제/자동 환급하지 않는다. 같은 part_attempt의 명시된 continuation은 새 소재 시도로 재계수하지 않으며, 새 소재 처리 시도는 새 ID와 남은 예산을 요구한다. 목표 양품 수량과 허용 시도 예산은 별도다. 본 판에서 run의 limit는 불변이다. 추가 시도가 필요하면 operator가 이전 run/처분을 참조하는 새 run과 시작 의도로 추가 범위를 지정한다. 새 run 생성도 관련 미결/소재/scope 제한을 없애거나 불명 작업을 재실행할 권한을 주지 않는다. 내부 node 하나마다 새 사람 승인을 요구하지 않는다.

예산의 consumed=limit는 **새 part/operation 생성 금지**를 뜻한다. 이미 계수한 part의 남은 node나 이미 생성한 operation의 전달/결과 확인은 그 이유만으로 차단하지 않는다. mandate EXHAUSTED는 새 예산 단위를 만들 수 없고 허용된 기존 시도의 실행/명시 continuation이 모두 끝났을 때 기록한다. 재시작에서 예산이 0이어도 명시된 기존 part continuation만 포함한 새 mandate를 만들 수 있다. 추가 part 생성은 계속 거부한다.

UI 창 닫힘·표시 연결 단절이 자동으로 생산 mandate를 취소하지 않는다. Runtime/Host/executor의 유효성, 현장 감독·담당자 조건이 별도다. 해당 조건이 필요한 envelope는 source와 상실 정책을 지정해야 하며, UI heartbeat로 사람의 물리 상주를 증명하지 않는다.

## 3. 일시적인 보류와 시작 의도 철회

| 사건 | 새 일반 작업 | mandate/다음 동작 |
|---|---|---|
| 정상 recipe의 WAIT_TARGET 미도달 | 관련 다음 작업만 대기 | ACTIVE 유지. 목표 충족 뒤 원래 범위 안에서 계속 |
| 아직 native 효과와 무관한 admission 관측의 일시적 부재 | 관련 dispatch 보류 | 상태/권한/구성이 안 바뀌고 근거가 복원되면 새 permit 평가 가능 |
| RPC 응답 유실, 같은 operation의 결과 조회 중 | 관련 후속 dispatch 보류 | 같은 key/ID 조회. 자동 재호출 금지. 상관 결과·현재 조건 회복 시 원래 run을 계속할 수 있음 |
| source 모순·결과 DISPUTED·알 수 없는 물리 변경 | 영향 closure 보류, 필요 현지 반응 | mandate 철회, 조사/개입 사례와 재시작 요구 |
| safety/emergency stop, 종류 미확인 stop | 영향 scope의 새 생산 차단 | mandate 철회. 기능별 reset/재확인 후 새로운 RestartRun |
| operator Pause/Stop, 사람 접근·수동 조작 | scope의 새 생산 차단 | mandate 철회. Pause만으로 물리 정지/접근을 확정하지 않음 |
| controller/Host/P/executor 재시작, 명령 owner 변경 | 관련 새 생산 차단 | 기존 permit·mandate 무효, journal/잔류 명령·물리 상태 재조정 |
| envelope/tool/지그/정책 등 관련 변경 | 영향 scope 차단 | 새 qualification/epoch와 새로운 시작 의도 |

단순 보류를 자동 해제하려면 **개입·보호 반응·권한/장비 세대 변경·미확인 외부 변화가 없다는 근거**가 필요하다. 그 연속성을 알 수 없으면 철회 경로다. PASS가 다시 나타났다는 이유만으로 사라진 연속성을 추정하지 않는다. 자동 safeguard reset/restart가 필요한 응용은 본 판에서 자동 지원으로 선언하지 않고 별도 검증/확장 요구로 기록한다.

block kind는 TRANSIENT와 LATCHED로 고정한다. 정상 WAIT_TARGET, 안전 반응 없는 admission 관측 보류, **원 invocation 결과를 조회 중인 단순 RPC 응답 유실**만 TRANSIENT가 될 수 있다. 마지막 경우도 물리 상태·권한 연속성 상실이나 안전 반응/사람 개입이 없다는 조건을 충족해야 한다. 나머지 철회/개입/보호 반응 사건은 LATCHED다. TRANSIENT 해제는 fresh evidence와 위 연속성 검사 후 명시적 ClearTransientBlock으로 기록하되 mandate를 새로 만들지 않는다. LATCHED는 조건값이 돌아와도 자동 해제하지 않는다.

TRANSIENT block의 영속 소유자는 P다. H에 그 block ID를 latch해 복제하지 않는다. H는 자신의 현재 local condition을 매번 검사하며 source가 회복되면 그 현재 결과가 바뀐다. H가 현지 보호 반응/latched 이상을 감지한 경우는 별도 LATCHED block이며 P의 transient 해제로 지워지지 않는다. FenceCell/ArmCell의 block ID는 LATCHED만 대상으로 한다.

## 4. operation의 DispatchPermit

RunMandate는 native 실행 token이 아니다. P는 각 operation에 대해 기존 T1과 셀 조건을 결합해 `DispatchPermit`을 발급한다. permit는 **operation ID/intent digest, purpose, cell/envelope/qualification revision, cell_epoch와 scope vector, grant/fence, Host boot, 만료 시간, 평가 evidence**에 결합한다. 다른 동작·다른 Host·다른 run에 사용할 수 없다.

permit 상태는 `ISSUED / CONSUMED / VOIDED / EXPIRED`다. 같은 permit의 중복 전송은 이전 receipt 조회이며 native 실행을 추가하지 않는다. 한 operation에는 유효한 permit가 최대 하나다. 아직 미전송이라는 Host의 영속 확인 없이 새 permit를 발급하지 않는다. 기존 SEND_ENTERED 작업에는 조회/취소/복구만 허용한다.

TTL은 envelope/profile가 제공하는 필수 값이다. 공통 고정 수치로 정지 성능을 주장하지 않는다. P/H는 공통 v1.0의 같은 host boot/CLOCK_BOOTTIME 조건을 검증하고, permit의 `expires_at`은 그 시간축으로 고정한다. Host restart 후 옛 permit를 복원하지 않는다. TTL 갱신은 없으며 만료 뒤 새 평가·새 ID를 요구한다.

## 5. Host의 최종 gate

Host는 다음을 **동일한 command gate**에서 검사하고 permit 소비·SEND_ENTERED 기록·native 제출 진입까지 직렬화한다.

1. peer/계약 capability, CellDefinition의 target/Host/cell membership 일치.
2. cell_epoch와 모든 의존 scope epoch, Host boot, grant/fence, purpose·operation/digest·permit ID, expiry 일치.
3. Host가 유지하는 block set과 관련 열린 개입/변경 scope가 허용하는지.
4. 해당 operation의 native mode·source freshness·MAINTAINED/ADMISSION 조건과 실제 장비 interlock 조건.
5. 기존 operation/cancel/native 결과와 충돌하지 않고 permit 미소비인지.

H는 permit 소비와 기존 delivery journal의 SEND_ENTERED를 **하나의 로컬 transaction**으로 저장한다. 기록 실패면 일반 native 효과 없음. native 호출이 이미 진입했으면 취소/epoch 변경으로 ‘없었던 호출’로 만들지 않는다. 지연/정지한 native 호출이 남아 있으면 새 owner나 접근 근거로 자원을 인계하지 않는다. 고속 sample은 별도 세션·단일 consumer 규칙을 적용하되 이 셀의 permit/epoch로 연 세션에만 속할 수 있다.

## 6. 무효화 전파와 보장 범위

P에서 latched invalidation 발견 시: 같은 transaction에서 block/새 epoch vector/관련 mandate 철회/permit 무효화/사건/outbox를 저장한다. 이어 영향 Host에 `FenceCell`을 전파한다. H는 그 gate에서 이전 scope epoch의 미진입 명령을 봉인하고 `FENCE_INSTALLED`를 durable ack한다. scope 정의가 불명확하면 셀 전체 경계로 확대한다.

P는 필요한 모든 Host의 ack 전에는 새 epoch를 운전용으로 활성화하지 않는다. H가 설치한 새 fence 이후에는 옛 permit로 새 native 진입이 불가능해야 한다. 이미 SEND_ENTERED인 호출·native queue·움직임/지지는 별도 조정하며 fence ack가 이들의 종료/접근 허가를 뜻하지 않는다.

H의 cell_epoch와 scope별 epoch는 durable 최대값이며 후퇴할 수 없다. 오래된 전체 vector로 현재 map을 덮어쓰지 않는다. 더 작은 cell_epoch 요청은 거부하고, 같은 cell_epoch의 각 scope 값은 저장값보다 작으면 STALE다. 같은 epoch의 재전송은 동일 payload/요청 ID일 때만 기존 receipt를 반환한다. FenceCell과 ArmCell을 같은 gate에서 처리하고, ArmCell은 정확한 현재 vector·명시된 block ID 집합만 대상으로 한다. 그 이후 생긴 block/새 사건은 지우지 못한다. 새 cell definition은 퇴역 scope의 tombstone과 새 membership을 명시하고 옛 permit를 다시 유효하게 만들지 않는다.

**분산된 관측의 한계:** P가 상실을 안 순간과 H가 fence를 적용한 순간 사이에는 지연이 있다. 그 사이 이미 발행된 permit가 H에 도착할 수 있다. RX는 P의 DB 변경 순간부터 모든 물리 동작이 즉시 사라진다고 보장하지 않는다. 위험에 필요한 즉시 반응은 검증된 현지 감지/보호 경로가 수행해야 한다. 그런 경로/시간 근거가 없으면 그 envelope를 qualification하지 않는다.

새 조건값의 도착만으로 H의 latched block을 해제하지 않는다. P의 명시적 새 평가·재시작과 epoch 동기화를 요구한다. TRANSIENT는 §3에 따라 해제한다. 외부 pendant/OEM 동작처럼 RX 밖에서 생긴 변화도 profile의 관측·현장 변경 절차에 따라 invalidate한다. 감지할 수 없는 조용한 변화가 있을 수 있음을 지원 제한으로 남긴다.

## 7. 회복·변경 때의 원자 경계

| transaction | 함께 기록할 것 |
|---|---|
| 시작 | StartIntent key, RunMandate, 현재 cell/envelope/qualification, run/executor 연결, 사건 |
| 작업 admission | 기존 T1 key/intent/activation/resource/outbox + 셀 조건 평가 + permit/현재 epoch |
| 보류/철회 | 영향 block, epoch와 mandate/permit 상태, 열린 개입/변경 사례, fence outbox, 사건 |
| 재확인 | 사용 evidence와 case revision, material/condition 무효화/새 평가, 처분·기록 |
| 재시작 | 예상 case/cell/run revision과 clearance 검사, 새 mandate, 새 activation/명시 continuation, 사건 |

기존 SQLite state DB 안에 같은 transaction으로 구성한다. 별도 셀 DB와 작업 DB 사이에 성공을 조립하지 않는다. H delivery/evidence journal은 기존대로 별도 사실 기록이며 두 DB와 물리 효과를 하나의 분산 transaction으로 주장하지 않는다. 모든 mutation은 key 중복을 CAS보다 먼저 처리하는 기존 규칙을 따른다.

## 8. 종류별 목적과 보호 예외

permit purpose는 `PRODUCTION / SETUP / RECOVERY`다. SETUP도 자동으로 가벼운 동작이 되는 것은 아니며 해당 envelope의 조건·역할을 만족해야 한다. RECOVERY는 특정 case/절차/step에 결합하고 평상시 생산 조건과 다른 **명시된 대체 조건**을 적용한다. `ignore_safety=true`, 임의 guard 생략, 일반 저속이라는 이유의 자동 허용은 금지한다.

읽기·기록 조회는 motion permit를 요구하지 않지만 접근권을 검사한다. 미리 할당된 현지 보호 반응은 저장/네트워크가 없어도 필요한 범위에서 동작한다. 이 예외를 생산·자동 재시작·임의 mode/torque 변경 경로로 확대하지 않는다. 정상 종료에서 지지 유지가 필요한데 장비 driver의 destructor를 호출하는 경로도 이 계약의 lifecycle 조건을 충족해야 한다.
