# 기존 작업 조회와 자원 인계

상태: 유한 작업의 `Operation.Reconcile` → P 영속 조회 계획 → Host 읽기 → 기존 T2/자원 인계 경로를 연결했다. S의 실제 BT `RequestHandover` 요청과 응답 유실·재시작 관측도 연결한다. 전체 복구 절차와 상주 운영 서비스의 완료 명세는 아니다.

## 책임과 의미

`Operation.Reconcile`은 기존 작업의 확인을 요청한다. 이 RPC의 응답은 그 시점의 `OperationView`다. 요청 접수, native 완료, 자원 해제는 서로 다른 사실이다. 요청 처리 자체가 새 생산 invocation이나 취소·물리 정지를 만들지 않는다.

현재 Executor 인증·셀 협상·셀 배정을 확인한 뒤 P에 작업별 조회 계획을 보존한다. control-session은 아직 지원하지 않는다. 이미 RELEASED인 작업은 현재 상태를 반환한다. 진행 중인 같은 작업의 요청은 하나의 PENDING 계획으로 합친다. 완료/확인 필요 상태 뒤 명시적인 새 조회는 새 계획 ID와 증가한 generation을 만든다. 이전 계획의 사건은 감사 원장에 남는다.

이는 Submit의 불변 접수 확인서 재생과 다른 조회 계약이다. Reconcile은 별도 결과 Receipt를 만들지 않고 최신 OperationView를 반환한다. S가 보존한 request key는 네트워크 시도 추적에 사용하며, P의 조회 계획 병합 단위는 operation이다. 원래 생산 명령의 key/invocation/예산을 변경하지 않는다.

## 저장과 조회 계획

`rx.internal.reconciliation-request.v1`은 id/operation/cell/host/generation/requested_by/state/issue를 가진다. 계획과 감사 사건은 같은 SQLite transaction에 저장한다. 상태는 다음과 같다.

| 상태 | 의미 | 완료 조건 |
|---|---|---|
| PENDING | 읽기 또는 추가 근거를 기다림 | 결과·인계 확인 전에는 완료로 바꾸지 않음 |
| COMPLETE | P가 실제 RELEASED를 저장함 | Work 처분을 현재 transaction에서 다시 확인 |
| ATTENTION | 권한·기록 연속성·지원 범위 확인 필요 | 자동 새 생산 요청으로 전환하지 않음 |

issue는 WaitingDispatch/WaitingResult/WaitingHandover/SourceUnavailable/ContinuityUnproven/PermissionChanged/Unsupported로 구별한다. 동일 상태의 반복 갱신은 사건을 늘리지 않는다. 계획 조회·관측 저장·결과 갱신은 현재 Host 계정/셀 범위와 등록 session을 확인하고, 계획 ID가 교체되면 옛 작업자는 변경할 수 없다.

## Host 조회 순서

1. 현재 Work가 RELEASED면 계획만 COMPLETE로 정리한다. 해제와 계획 정리 사이에 프로세스가 종료되어도 재실행 시 이 단계에서 회수한다.
2. 결과가 없으면 실제 EMIT_ENTERED/DELIVERED outbox가 있는지 확인한다. 아직 발행 근거가 없으면 WaitingDispatch로 남긴다.
3. `GetReceipt`와 `Host.Reconcile`로 기존 invocation을 조회한다. receipt는 기존 처리기, evidence batch는 동일 T2 inbox로 적용한다. 없는 receipt나 응답 timeout을 미실행 증거로 바꾸지 않는다.
4. 미결 결과는 WaitingResult다. 무결성 분쟁·UNRESOLVED는 ATTENTION이다.
5. terminal 결과 이후 `WatchObservations`에서 no-pending/control/support 세 관측을 읽는다. 읽은 원문을 먼저 보존하고, 별도 transaction의 기존 `ReleaseResources`에서 실제 인계 조건을 판단한다.
6. 현재 operation/cell revision과 세 관측이 유효할 때에만 resource holder와 Work 처분을 함께 변경한다. 후속 계획 COMPLETE는 해제의 원인이 아니라 해제 사실을 확인한 상태다.

조회 worker는 Prepare/Authorize를 직접 호출하지 않는다. 기존 PREPARED receipt의 처리는 원래 T1의 유효한 permit에 한하여 기존 dispatcher가 Authorize를 이어갈 수 있다. Pause로 봉인된 permit을 다시 허용하지 않는다.

## 관측 보존과 인계 판정

인증된 Host의 현재 boot에서 온 세 관측을 원래 ID와 내용으로 보존한다. false, 품질 불충분, 오래된 관측도 조회 사실로 남긴다. 보존은 조건 충족 판정이 아니다. 동일 ID의 다른 내용은 기존 관측을 덮어쓰지 않으며 충돌 사건과 셀 closure 제한을 먼저 commit한 뒤 무결성 오류를 반환한다.

해제에는 세 종류 모두의 true 값, 품질·원천 age 보장, 최대 age/불확실성, 동일 operation/invocation/profile/device session/Host boot, 현재 epoch/scope 및 cell readiness가 필요하다. 완료 근거보다 이전의 관측도 거부한다. false나 stale 관측이면 자원을 보유한 채 새 관측을 기다린다. 단순 성공 결과·SDK 응답·정지 표시만으로 인계하지 않는다.

## S 요청 journal과 재시작

S는 visit/node/ReconcileOperation에 body와 key를 먼저 저장하고, EMIT_ENTERED commit 후 RPC를 호출한다. 응답은 `ReconciliationAccepted`로만 보존한다. P snapshot의 실제 RELEASED는 별도 `ObservedTarget::Released`와 read basis로 기록한다.

RPC 응답 유실 뒤 P가 해제했더라도 S의 원래 PENDING 응답 상태를 성공 응답으로 덮어쓰지 않는다. 재시작의 `recover`는 같은 mapping과 실제 해제 관측을 회수한다. 계속된 BT 요청도 이미 접수된 조회를 매번 새 RPC로 보내지 않는다. 조회에는 CAS 변경이 없으므로 ABORTED를 근거로 새 key/body를 생성하는 경로를 금지한다.

## 검증 범위와 남은 연결

- core: 진행 중 요청 병합, generation 교체와 옛 ID 거부, 해제 전 COMPLETE 거부, false 관측 보존, 관측 ID 충돌 시 제한.
- Host 통합: 실제 mTLS/별도 모의 Host 프로세스에서 성공 뒤 첫 support=false를 거부하고, 후속 근거로 해제한다. 두 소재의 native effect는 두 번을 유지한다.
- S 통합: 실제 Linux BT.CPP 요청 → 별도 S journal/worker → P mTLS 조회. 정상 응답과 commit 후 응답 유실, 이후 P 해제 및 새 S boot의 관측을 확인한다. 이 시험의 Host 완료·인계 근거는 명시적인 합성 fixture이며, 앞의 Host 통합 시험과 범위를 구별한다.

조회는 pass당 최대 4계획, Host 읽기당 2초 timeout, 100ms–5초 backoff와 제한된 재시도 메모리를 사용한다. 현재 pending 조회는 scan 기반이며 Host 조회가 dispatcher pass와 직렬이다. 큰 원장에 대한 index/paging과 Fence 전달 지연을 제한하는 별도 스케줄링은 운영 서비스 구현 전에 해결해야 한다. 이 수치를 실시간 응답 보장으로 사용하지 않는다.

`Host.Reconcile`의 최대 128개 연속 batch만으로 긴 backlog를 모두 회수한다고 주장하지 않는다. Host의 영속 background Evidence.Publish/ack 경로가 함께 필요하다. 현재 계획 ATTENTION의 상세 조회/UI 표시, 작업자 재조회·취소/복구/clearance, control-session, 상주 loop·part coordinator·liveness는 미완료다. 물리 셀은 계속 NOT_COMMISSIONED다.
