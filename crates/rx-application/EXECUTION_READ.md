# 복원 자료와 현재 실행 상태 조회

상태: shared DTO, P의 현재 read cut, 별도 S client, C++ frame 입력·시계 검사에 연결했다. S의 영속 요청·유한 operation worker까지 연결했고 전체 요청 종류/daemon/자동 복구는 후속이다.

## checkpoint와 live snapshot

Checkpoint는 해당 Run revision에 저장한 과거 복원 자료다. 그 뒤 operation 결과·자원 상태·관측 age가 바뀔 수 있으므로, checkpoint의 EXECUTING만 보고 현재 요청 가능 상태로 사용할 수 없다.

`Engine::execution_snapshot`은 한 transaction에서 현재 executor 접근권/definition 협상, Run/셀 epoch·scope, 현재 process progress, checkpoint와 원장 위치를 읽는다. 반환 위치는 그 transaction의 실제 control cut이다. 요청 가능 여부와 허용되지 않는 domain reason을 함께 담는다. 저장소 무결성/조회 실패를 단순히 ‘허용 안 됨’으로 숨기지 않는다.

조건이 새 패킷 없이 시간 경과로 낡을 수 있으므로 active_run은 maintained conditions를 실제 처리 시점에 다시 평가한다. 새 activation/part/operation admission도 이 공통 검사에 들어간다. 이것은 진행 중 장비의 독립 local protection이나 supervisor aging/liveness monitor를 대신하지 않는다.

## 공유 데이터와 책임

Run/Purpose/RunState, ProcessCheckpoint/WaitWindow, checkpoint/activation/slot DTO를 ROS·BT·I/O 비의존 `rx-process-contract::execution`으로 이동했다. 기존 checkpoint schema/필드·정규화 bytes 의미는 유지한다. S SDK에 data model을 공유하며 P의 Engine/transaction/권한 writer는 수출하지 않는다.

`rx.execution-snapshot.v1`은 현재 실행기 조회용 schema다. installation/store generation/runtime boot, caller session, control sequence, cell definition/envelope·revision/epoch/scopes, P clock bounds, run/checkpoint, requested visit, resolved ArtifactRef, 현재 process progress와 admission flag/reason을 포함한다.

shared validator는 schema/clock 범위, run/checkpoint/visit, actual resolved bytes/hash/schema/size, 원장·scope position, 현재 authority flag와 Run의 관계, checkpoint/progress 결정 일치, activation/slot/operation의 완전한 연결과 intent digest를 검사한다. 복원 자료를 caller가 조립한 작업 결과로 대체하지 않는다.

## 선택적 읽기 binding

Frozen RunView 하나에 현재 process-specific progress를 임의 필드로 덧붙이지 않았다. 별도 [`rx.executor.v1` 읽기 binding](../../spec/executor/v1/README.md)을 두고, 각 호출이 정확한 binding manifest hash를 제시하도록 했다.

- GetSnapshot: 현재 검증된 graph/run/visit의 read cut을 typed canonical bytes와 정확한 ArtifactRef로 반환.
- GetArtifact: 해당 run 소유 checkpoint 또는 현재 run과 일치하는 resolved process만 반환. 임의 경로/URL·일반 blob 쓰기·proposal 제출 없음.

기존 base/cell peer의 필수 협상 규칙과 mutation을 그대로 유지한다. 추가 읽기 API는 Workflow.CommitCheckpoint나 Cell.SubmitOperation을 대체하는 우회 경로가 아니다. 추가 protobuf/DTO/validator/의미 문서를 별도 manifest에 고정하고 Cargo build와 점검 도구에서 대조한다. frozen 규범8개와 manifest는 변경하지 않는다.

Payload 최대1,000,000 bytes, 전체 gRPC1 MiB. 초과를 부분 snapshot으로 잘라 반환하지 않는다. 현재 source 수집은 scan을 사용하며 대규모 paging/index/retention은 아직 미완료다.

## 시간과 C++ 경계

P snapshot의 유효 범위는 최대100 ms이며 session expiry도 넘지 않는다. 이는 현재 상태 조회의 age 상한이지 물리 동작 허가가 아니다. 새 T1에서 현재 권한·조건·자원을 다시 검사한다.

S client는 인증서/양쪽 기본 협상/읽기 binding, 데이터 hash/size/schema, 동일 설치·저장 세대·runtime·cell/session/원장 위치, 실제 resolved process와 모든 상태 연결을 확인한다. P read time을 요청 전후의 신뢰된 동일 호스트 시계 범위와 대조한다. 별도 request-send Instant bound도 함께 적용한다.

Linux adapter는 kernel boot ID와 CLOCK_BOOTTIME을 사용한다. 호스트 suspend 동안 멈출 수 있는 local steady deadline 하나만으로 현재성을 판단하지 않는다. C++ frame에는 원래 source clock ID/시작/만료와 짧은 local 잔여 시간이 모두 들어간다. Context는 publish/tick/queue handoff에서 source clock을 다시 확인하며 불일치·만료·clock 오류면 요청을 내지 않는다.

C++ decoder는 중복/추가 JSON key, 숫자로 보낸 uint64, overflow, 알 수 없는 enum, 잘못된 ID, 중복 eligible node와 시간 범위를 거부한다. 새 epoch/session/digest를 기존 Context에 자동 채택하지 않는다. malformed frame은 기존 pause 규칙에 들어간다. 시험용 in-process synthetic Frame과 실제 IPC packet의 source-clock 의무를 구별한다.

## 검증과 다음 단계

- 실제 P 데이터에서 branch 결정·원장 cut·run-owned artifact와 fresh/aged maintained condition을 대조.
- 새 S 프로세스가 mTLS로 접속하면 기존 E 세션/Run 권한을 철회하고, 이전 작업·slot을 복원하되 새 admission은 false임을 확인.
- 그 client가 만든 Frame을 실제 Linux C++ BT 엔진에 입력하여 무허가 재실행/성공 처리가 없음을 확인.
- source clock이 만료되어도 local steady deadline은 남아 있는 반례, epoch 자동 채택 거부, duplicate/unknown/overflow 입력 거부.
- 실제 Linux에서 C++ 및 Rust CLOCK_BOOTTIME adapter 실행. Linux 네이티브 Rust client 시험은 고정 toolchain과 기존 검증 이미지에서 offline으로 실행했다.

시계·operator/qualification/장비 상태는 명시적인 simulation fixture다. pending-key journal·유한 작업/Pause/인계 worker와 P branch/wait/CommitCheckpoint를 후속 단계에서 연결했다. S 분기/대기 worker는 연결했으며 product BT daemon/Frame loop·전체 recovery/liveness·두 제품 image·설치/복원/인수는 미완료다.

## 구성 교체 이후의 과거 Run

[작업별 불변 구성](PROCESS_APPLY.md)을 사용한다. resolved/process는 해당 Run이 생성될 때의 구성에서, 현재 cell revision/epoch/scopes와 admission은 현재 셀에서 읽는다. 완료된 옛 공정을 최신 recipe로 해석하지 않는다. 새 실행 admission에는 전체 구성 일치를 요구한다.
