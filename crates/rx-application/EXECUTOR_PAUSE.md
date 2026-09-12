# Executor PauseRun과 허가 철회

상태: 실제 Workflow.PauseRun, S의 영속 pause 요청, C++ halt fixture에 연결했다. Pause 응답은 native 취소·정지·접근 허가·자원 인계 완료가 아니다.

## 적용 범위

현재 Host permit/Arm/fence는 cell epoch와 scope에 묶여 있다. 이미 진행 중이거나 Arm 준비 중인 run의 중단에서 local Run.state만 바꾸면 기존 Host 허가의 정리가 불명확해진다. 따라서 이 구현은 해당 cell과 검증된 공유 scope/resource closure를 함께 철회한다.

- 실행 중인 origin run은 PAUSED, 영향 closure의 다른 진행/준비 run은 RECOVERY_REQUIRED다.
- cell/scope epoch를 올리고 ExecutorPause LATCHED block을 남긴다. active mandate와 미사용 permit를 철회하고 fence outbox를 함께 기록한다.
- 아직 Arm/mandate/permit가 없는 PREPARED run은 자체 상태만 PAUSED로 바꾼다. 다른 cell 작업을 불필요하게 fence하지 않는다.
- 이미 PAUSED/RECOVERY_REQUIRED/COMPLETED/ABANDONED인 run은 새 fence나 상태 하향을 만들지 않는다. 이미 완료한 상태를 PAUSED로 덮어쓰지 않는다.
- 예산과 part/activation/operation identity는 유지한다. 자동 재시작이나 예산 환급은 없다.

이 Pause는 explicit halt/중단 요청의 제한 경로다. 정상 WAIT_TARGET·일시적인 조회 지연을 이 API로 바꾸지 않는다. 그 상태는 기존 transient/연속성 규칙을 따른다. 실제 pause 해제와 run 재시작은 별도 명시적 조건·재확인 절차의 대상이며 아직 전체 restart API가 완료되지 않았다.

## 검사와 transaction

현재 executor mTLS session·계정 역할·cell definition 협상·배정을 확인한 뒤, key와 전체 PauseRun payload(expected run revision 포함)를 비교한다. 신규 요청은 run CAS를 검사하고, EXECUTING run은 현재 owner session이 일치해야 한다. 같은 key/body는 원래 응답을 반환한다.

상태/epoch/block/mandate/permit/outbox 및 control event/checkpoint를 하나의 transaction에서 처리한다. origin PAUSED를 만들기 위해 별도 두 번째 Run update를 하지 않는다. 기존 invalidation을 parameterize해 같은 commit의 최종 상태를 capture한다.

`Finalized`는 control capture 후 같은 transaction에서 완성된 projection을 읽고 요청 결과만 저장할 수 있다. core put/append/outbox mutation 기능은 노출하지 않는다. 이를 통해 PauseRun의 응답도 최종 RunSnapshot과 같은 cut에서 cache하며, read-after-commit으로 다른 시점의 상태를 최초 응답에 섞지 않는다.

## 전달 상태와 결과

- NEW인 operation delivery는 VOIDED로 봉인한다. native 미발행을 P가 증명할 수 있는 경우에만 NOT_EXECUTED 근거를 기록한다.
- 이미 EMIT_ENTERED이면 Pause만으로 CANCELED/NOT_EXECUTED/성공을 만들지 않는다. 미결 전달은 UNKNOWN/격리와 조정 경로에 남긴다.
- issued permit는 VOIDED가 되므로 늦게 온 PREPARED receipt가 새 Authorize outbox를 만들지 못한다.
- pending start attempt는 REJECTED이고 늦은 Arm acknowledgment는 run을 다시 시작하지 못한다. 미발행 Arm도 현재 attempt 검사를 통과해 emit할 수 없다.
- late correlated native result는 계속 수용한다. 실제 결과가 확인되어도 resource handover는 별도이며 격리/지지를 임의 해제하지 않는다.

Fieldbus stop이나 robot cancel을 대신 호출한 것으로 기록하지 않는다. Fence 전달과 이미 진입한 작업의 조사/취소·local protection은 각자의 계약을 유지한다.

## S 요청과 반복 중단

S journal에 PauseRun body와 전체 frozen RunView 응답을 기록한다. pause logical key는 같은 run/visit/root뿐 아니라 원래 executor session/epoch를 포함한다. 이 optional control identity가 없는 기존 operation key의 canonical 표현은 유지한다.

따라서 같은 pause의 응답 유실은 같은 key로 회수하고, 이후 명시적으로 다시 허가된 다른 context의 새 halt는 별도 요청이 된다. 과거 halt로 더 새 epoch의 실행을 자동 중단하지 않는다. 새 snapshot에서 이미 제한된 run이 확인되면 observation으로 기록하며 받지 못한 RPC reply를 만들지 않는다.

C++ `Executor::halt`가 만든 PauseExecutor request를 S worker가 검증해 이 경로로 전달한다. RPC 응답 후에는 C++/S 자체 판단으로 동작 재개나 물리 정지 완료를 선언하지 않는다.

## 검증

- core commit 직전 실패/commit 후 응답 유실에서 RunSnapshot·fence·무효화·key 결과 원자성.
- shared cell closure 반영, origin Run revision 한 번 증가, part/budget 보존과 같은 key의 응답 불변.
- 미발행 작업의 봉인, 늦은 PREPARED 뒤 Authorize 생성 금지, Arm 준비 중 pause 뒤 late acknowledgment 거부.
- 이미 emit된 요청은 취소로 단정하지 않고 후속 상관 결과를 받아 기록.
- PREPARED/no-Arm run은 cell epoch를 바꾸지 않음.
- 실제 mTLS PauseRun과 C++ halt→S journal→P 처리, pause 응답 유실과 새 boot recovery에서도 pause key/미수신 상태 보존.

시험의 clock/qualification/Host/operator는 명시적인 simulation fixture다. 실제 장비 정지 시간·안전 접근·현장 복구 인수는 별도이며 이 구현의 통과로 주장하지 않는다.
