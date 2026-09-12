# 실행기의 소재 시도·단계 활성화 요청

상태: 실제 mTLS RPC와 단일 writer/SQLite transaction에 연결했다. 전체 BT worker·장비 제출·checkpoint 변경 API의 완료를 뜻하지 않는다.

## 요청과 변경 소유자

| RPC | 주요 입력 | P가 기록하는 결과 |
|---|---|---|
| Cell.BeginPartAttempt | 현재 CellCall, run, mandate, expected budget revision, optional cell revision | PartAttempt의 ID/ordinal/revision, run 소유 예산 소비와 run revision, 원장·복원 상태 |
| Workflow.ResolveActivation | 현재 CallContext, key, run, node, visit, expected run revision | 유일 activation 또는 기존 mapping, 새 할당이면 run revision과 복원 상태 |

RPC adapter는 메시지 형식·revision 위치·필수 key를 확인하고 typed command를 writer에 전달한다. JSON/Protobuf 안의 역할·단말·승인 bool로 실행 신원을 만들지 않는다. 내부 `ProcessingContext`의 시각은 writer가 처리할 때 얻은 P 시각이다.

공개 요청을 위한 `executor_requests`는 요청 envelope와 영속 결과를 처리한다. 실제 part 생성·예산 소비 및 activation 생성·적격성 판단은 기존 workflow와 같은 transition 함수를 사용한다. 별도 SQL 연결, 별도 작업 번호 allocator, 두 번째 업무 상태기계를 두지 않는다.

## 검사와 저장 순서

1. 현재 P session/계정 Executor 역할, 계정 셀 범위, CellDefinition 협상, 해당 셀의 executor 배정을 확인한다. 새 boot나 역할 회수 후에는 과거 key로 이 검사를 건너뛰지 못한다.
2. 요청 key와 typed 업무 payload 전체의 fingerprint를 비교한다. 이미 적용된 같은 요청이면 당시의 저장된 응답을 반환한다.
3. 새 BeginPartAttempt는 run/cell 관계와 명시한 mandate, optional cell revision, 현재 run/mandate/qualification/조건과 budget revision·잔여 예산을 검사한다.
4. 새 ResolveActivation은 run/node/visit의 기존 mapping부터 회수한다. 새로운 mapping이면 run revision·현재 실행 권한·실제 part ordinal·공정 frontier를 검사한다.
5. core 변경, 요청 결과, 원장, 현재 projection과 checkpoint artifact는 같은 repository transaction에서 commit한다. 오류가 나면 함께 rollback된다.

응답 유실 뒤 같은 key/body를 반복해도 예산을 다시 소비하거나 새 activation을 만들지 않는다. 조회를 한 뒤 새 revision을 얻었다고 해서 같은 key의 body를 바꾸지 않는다. 바뀐 mandate/visit/revision은 `KEY_CONFLICT`이며 gRPC `ALREADY_EXISTS`로 반환한다. 새 의도에는 새 key가 필요하다.

새 key라도 이미 존재하는 `(run,node,visit)`는 기존 activation을 반환한다. 기존 mapping을 회수하는 것과 새 할당의 CAS를 구별하므로, 그 mapping이 생기기 전의 revision으로 다시 조회성 요청을 보내도 새로운 ID를 만들지 않는다. 다만 현재 신원/셀 접근 검사는 유지한다.

CallContext의 call ID와 인증 session 자체는 업무 fingerprint에 넣지 않는다. key는 installation/client namespace/method에 속한다. 업무 입력과 CAS 값은 fingerprint에 포함한다. 기존의 간단한 trusted composition helper와 공개 typed 요청의 payload가 다르면 같은 key를 교차 재사용하지 않는다. 서로 다른 payload를 같은 요청으로 처리하지 않으며, 공개 BFF를 추가할 때는 이 typed 경로를 사용해야 한다.

## revision과 응답

BeginPartAttempt 응답에는 해당 commit의 실제 PartAttempt record revision을 저장한다. 나중의 part 처분을 조회해서 과거 요청 응답에 새 revision만 붙이지 않는다. `material_id`는 아직 모델에 결합한 식별이 없으므로 absent이며, 임의 소재 UUID를 발급해 확인된 실물로 표시하지 않는다.

ActivationView의 slot/operation/intent 연결은 Work와 run/part/cell 관계까지 확인해 같은 transaction에서 만든다. 과거 key의 저장된 응답과 최신 GetRun을 구별한다. 이후 생성된 자식 mapping은 GetRun의 새 checkpoint에서 회수한다.

ProtoJSON의 기본 숫자 표현을 쓰지 않고 기존 frozen codec을 적용한다. positive visit/budget/run revision을 요구하고, CellCall의 base expected_revision을 객체별 expected_*와 중복해서 받지 않는다.

## 검증 범위

- 실제 저장 직전 rollback 및 commit 직후 응답 유실에서 part 수·예산·activation·checkpoint의 일관성.
- 같은 key와 바뀐 mandate/budget/visit의 충돌, 새 key의 기존 activation 회수.
- 예산을 소진한 뒤에도 동일 Begin 요청은 원래 part를 반환하며 추가 소비가 없음.
- 역할 회수 후에는 cached part/activation 응답도 권한 검사를 우회하지 못함.
- 실제 TLS socket/단일 writer/SQLite에서 Session→Cell 협상→모의 operator 시작→BeginPartAttempt→ResolveActivation→GetRun의 연결.

통신 시험의 operator 단말, qualification, Host 준비/Arm acknowledgment는 명시적인 simulation fixture다. 이 시험은 Work를 만들거나 native device를 호출하지 않는다. 실제 장비 전달은 별도 Host E2E 범위다.

## 다음 연결

[Cell.SubmitOperation의 유한 작업 경로](EXECUTOR_SUBMISSION.md)를 활성화했다. run/activation/slot/part와 parent mandate, 중첩 CallContext, 두 객체 CAS를 같은 T1에 연결하고, 실제 원장 위치의 [최초 접수 확인서](ADMISSION_RECEIPT.md)를 반환한다. 연속 제어·복구 제출과 아래 공정 제어 연결은 후속이다.

P의 branch/wait 결정·Workflow.CommitCheckpoint의 release schema/CAS, E artifact 확보·C++ Frame/유한 작업/Pause/인계 worker·pending key 복원은 후속 단계에서 연결했다. S 분기/대기 worker는 연결했으며 전체 복구는 남았다. 현재 두 요청의 동작을 전체 executor 실행·복구·운전 qualification으로 확대해 표현하지 않는다.
