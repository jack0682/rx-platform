# P 자격 발급·Host 전달·전역 활성화

상태: phase54 구현 초안. [재검증 근거·독립 검토](REQUALIFICATION_REVIEW.md), [Host 자격 수용](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/QUALIFICATION_ACCEPTANCE.md)을 P의 영속 발급/전송/활성화와 연결한다. 실제 현장 시험을 대신하지 않으며 시험 fixture는 SIMULATION 범위다.

## 세 가지 다른 사건

1. **발급/PENDING**: 현재 승인과 원본 패키지·정책을 재검증하고 셀별 qualification ID 및 Host task를 원장에 고정한다. 아직 Cell.qualification을 설정하지 않는다.
2. **Host 수용**: Host가 현재 전체 cohort/문맥/세대에 자격을 연결하고 receipt를 보관한다. block과 비활성 Arm 상태는 유지한다.
3. **활성화/ACTIVE**: P가 모든 Host의 최근 수용과 현재 검토/권한/원본을 다시 확인하고 qualification·commissioning·승인된 block 해제를 원자적으로 기록한다. 여기서도 Run/Arm/permit/native 명령을 만들지 않는다.

그 뒤 작업자의 별도 StartRun이 현재 조건·역할·Host lease와 자격을 검사한다. Arm 완료, mandate, 개별 operation의 permit와 Host/native guard가 뒤따른다. 자격 활성화는 작업 실행과 같은 뜻이 아니다.

## 정책 v2와 기존 기록

`rx.requalification-policy.v2`는 각 Profile에 비어 있지 않은 `purposes` 집합을 요구한다. 값은 PRODUCTION/SETUP/RECOVERY 중 명시한 것만 허용한다. P의 일반 StartRun/active Run 검사와 Host의 수용 범위/permit 검사에서 이를 적용한다. RECOVERY의 모든 실제 실행 기능이 구현됐다는 뜻은 아니다.

기존 v1은 purposes가 없는 상태로 읽고 재검증 자료/검토 기록을 유지할 수 있다. 빈 집합은 serialize 시 생략하여 과거 canonical bytes 의미를 보존한다. v1 검토에서 운전 목적을 추론하지 않으며 발급 worker가 거부한다. 새 목적을 추가하려면 v2 정책으로 새 요청·결과·독립 검토를 거친다.

## 발급 입력과 근거

IssueRequest는 review/cell, report revision/digest, decision revision, 전체 영향 셀의 expected_cells revision map, 셀별 clear_blocks ID 목록을 받는다. 역할은 현재 등록 단말의 ReleaseManager이며 사용자·단말 권한 교집합이 전체 범위를 포함해야 한다.

preflight와 commit에서 검토의 현재 policy/Runtime boot/적용 기록·셀 구성/epoch/scope/block, 최신 승인/CAS와 정확한 fence ack를 검사한다. 영향 Run은 COMPLETED/ABANDONED이어야 하고, 미확정/분쟁 work·보유/격리 자원·관련 열린 사건이 없어야 한다.

worker는 writer 밖에서 저장된 재검증 원본·서명과 현재 pinned v2 policy를 다시 검증한다. 최초 공정 검토가 가리킨 원본 package object도 현재 package policy/Store에서 다시 검증한다. 실제 StoredPackage owner/object/policy와 private Prepared proof가 일치해야 한다. 원본 내용이 없거나 현재 trust와 다르면 발급/활성화를 진행하지 않는다.

발급 commit은 Batch/셀별 새 qualification ID/revision, 제한된 purpose/의존 자료/근거/limitations, Host별 task ID·incarnation, sender 신원, review slot과 사용자 요청 결과를 한 transaction에 기록한다. 같은 review/report/decision slot에 다른 발급 의도를 덮어쓰지 않는다. 같은 key/body는 원래 ID들을 회수한다. PENDING 발급의 sender 세션이 끝났다면 현재 등록 단말의 ReleaseManager가 같은 발급 내용과 새 key로 재승인할 수 있다. worker가 원본/정책을 다시 검증하고 sender와 Batch revision만 갱신하며 qualification/task/request ID와 본문은 바꾸지 않는다.

## 해제할 block의 출처

새 configuration preparation, P apply, requalification preparation 및 이 자격의 suspension/runtime-restart에서 생성한 block에 `(block, cell, change, reason)` 소유 원장을 기록한다. 발급 입력이 그 소유와 현재 실제 block을 만족해야 한다. reason 이름만으로 block을 지우지 않는다.

manual Hold, 다른 사건/변경, 출처 원장이 없는 옛 block은 해제 대상에 넣을 수 없다. 해제 목록을 비우거나 확인된 일부만 선택할 수 있으며 나머지는 유지한다. 따라서 qualification이 활성화돼도 남은 block 때문에 Ready/StartRun은 거부될 수 있다. 이전 초안 데이터의 출처 불명 block을 자동으로 추정해 해제하는 migration은 제공하지 않는다.

최종 활성화는 정확한 목록만 P에서 제거한다. Host에 실제 남은 block의 해제는 **후속 명시적 StartRun의 Arm**에서 수행한다. Arm outbox에는 자격이 승인한 해제 가능 ID 집합이 고정된다. client는 현재 Host block을 읽고 모든 ID가 그 집합 안에 있을 때만 현재 남은 ID를 전송한다. 이미 해제된 ID를 다시 요구하지 않는다. Host는 부분 해제/남은 block이 있는 Arm을 성공으로 처리하지 않는다.

## 영속 Host task와 불명 결과

Task 상태는 AWAITING_SNAPSHOT → PREPARED → SEND_ENTERED이며 receipt의 ACCEPTED/NOT_ACCEPTED와 별도다. worker는 현재 Host snapshot과 P 적용 당시의 원래 process-context request/receipt sequence를 대조한 뒤 정확한 Request/digest를 저장한다. 전송 진입 commit이 끝나야 RPC를 반환/실행한다.

- 전송 전 저장 실패: Host에 보내지 않는다.
- 전송 진입/Host commit 뒤 응답 유실: 원래 ID를 먼저 Lookup한다.
- receipt 없음: NOT_ACCEPTED로 추정하지 않는다. 같은 Host 세대·Binding·전체 문맥 및 현재 발급/송신 권한을 재검사한 경우 이 멱등 metadata 요청만 동일 본문/ID로 재전송할 수 있다.
- 첫 receipt 이후 소실/모순: 원래 사실을 보존하고 disputed를 latch한다.
- 이전 미확정/분쟁 전송: 새 발급으로 우회하지 않는다. 사실 조회는 현재 해당 Host 신원으로 계속 가능하다.

Host별 accepted 수, 부분 수용, 결과 불명과 현재 문맥을 조회한다. 일부만 수용되거나 현재성이 증명되지 않으면 전역 활성화를 하지 않는다. NOT_ACCEPTED의 자동 대체 발급/취소·세밀한 운영 복구 UI는 후속이다.

## 최종 활성화

Activate 입력은 batch/cell, batch revision, 전체 셀 revision map이다. 현재 등록 단말 ReleaseManager가 별도로 요청한다. 원본 package/재검증 근거·정책을 worker에서 재검증하고 commit에서 권한·review/decision·quiet barrier·block 소유를 다시 확인한다.

모든 Host receipt는 ACCEPTED, 무분쟁, 현재 boot/journal/session/Binding/process context/epoch와 일치해야 한다. 조회 시작 시각과 observation digest를 P writer 메모리 proof에 기록하며3초 이내만 사용한다. 이 freshness를 재시작 후 복원하지 않는다. 원격 snapshot은 지속 물리 안정성의 보증이 아니고 native 진입의 현재 session/guard가 추가로 필요하다.

하나의 transaction에 셀별 Qualification/COMMISSIONED/SETUP, 승인된 P block 제거, certificate→batch 연결, Batch ACTIVE/history/시간, Change QUALIFIED_ACTIVE와 요청 결과를 저장한다. cell/scope epoch는 Host가 확인한 값을 그대로 사용한다. source package/정책/서명 실패나 CAS 경합, 부분 Host 결과에는 아무 셀도 부분 활성화하지 않는다.

활성화 응답 유실 뒤 같은 key는 당시 결과를 회수한다. 그 응답만으로 현재 authority가 살아 있다고 판단하지 않는다. 현재성은 GET batch 및 새 조작의 권한/조건 검사에서 확인한다.

## 유지·정지·재시작

새 admission은 certificate의 구성/epoch/scope, 현재 등록 정책/패키지 service, 해당 검토 결정 및 Host 수용 문맥을 확인한다. 활성화 후 통신 오류는 현재 사용 가능성과 구별한다. 같은 자격 receipt의 실제 소실/모순·Host 문맥 상실은 cohort를 suspend하고 새로운 epoch/fence와 제한을 기록한다. 기존 receipt를 지우거나 결과를 새로 만들지 않는다.

명시적 Suspend, policy/package registration 변경과 Runtime 재시작은 활성 자격을 history로 보존하고 Cell에서 제거하며 Batch SUSPENDED/Change APPLIED_UNQUALIFIED로 기록한다. 정지된 같은 Batch를 다시 활성화하지 않는다. 재시작은 새 검토 준비/세대/Host 확인을 요구한다. 사용자 Hold 같은 별도 제한을 이 경로가 임의 해제하지 않는다.

Host 자체도 수용 receipt의 모든 cohort 구성원과 현재 accepted record/epoch/process context를 검사한다. 같은 Host의 한 구성원이 바뀌면 다른 구성원의 옛 자격으로 Arm/native 진입을 허용하지 않는다. 실제 장치 session도 수용 시 값과 달라지면 거부한다.

P의 원장 변경과 모든 Host의 실제 fence 사이에는 분산 지연이 있다. 즉각적인 물리 정지/지지는 검증된 현지 보호 경로의 책임이며 이 구현이 네트워크/DB만으로 보증하지 않는다.

## fence 이후 기존 lease 갱신

확인된 fence가 현재 P cell epoch/scope와 같을 때만 HostRegistration의 그 projection을 갱신한다. 오래된 ack는 현재 세대를 뒤로 돌리지 않는다. grant/만료/Arm은 이 ack로 갱신하지 않는다.

renewal은 처음 연결한 동일 Host boot/journal/producer/source session, 동일 grant ID/fence/자원 범위와 definition을 유지해야 한다. 현재 셀의 필요 자원이 원래 grant 안에 있어야 한다. 그 조건에서 새 fence를 확인한 epoch를 허용하며 initial LinkPlan/fence receipt를 다시 만든 것으로 처리하지 않는다. 만료한 grant를 새로 획득하거나 범위를 넓히지 않는다.

## API와 구현 배치

| API | 의미 |
|---|---|
| POST `/api/v1/qualification-activations` | 현재 단말 ReleaseManager: 발급 준비/원본 재검증/영속 Batch |
| GET `/api/v1/qualification-activation?cell=...&id=...` | Engineer/Verifier/ReleaseManager: Batch/Host별 결과·현재성 |
| POST `/api/v1/qualification-activation/activate` | 현재 단말 ReleaseManager: 전역 활성화 |
| POST `/api/v1/qualification-activation/suspend` | 현재 단말 ReleaseManager: 명시적 제한/정지 기록 |

기존 `{request_key, command}`와 HTTPS 인증·CSRF를 재사용한다. public API에서 임의 qualification ID나 Host Request 본문을 받지 않는다. runtime의 qualification worker는 Host connection의 수명 안에서 현재 P task만 수행한다. read proof/네트워크 대기는 단일 DB writer의 I/O 대기로 구현하지 않는다.

application은 `qualification_activation`의 issuance/host_tasks/activation 모듈과 공유 검사로 나누었다. runtime package worker가 원본 검증을, Host client worker가 전송/조회·중단을 담당한다. 이 경로가 구성된 실행 파일은 VERIFIED_REQUALIFICATION authority로 표시한다. 기존 직접 qualification port는 startup 값/Boolean으로 자격을 생성하지 않는다.

## 검증과 남은 범위

[phase54 기록](../../../references/implementation/phase54_checks.json)을 따른다. 원장 시험은 발급/활성화 원자성·응답 유실, 같은 ID, 미확정 Lookup, 현재 문맥 재검사, policy 회수, receipt 소실, 부분 Host 수용, manual Hold 보존, Runtime 재시작 및 v1 목적 추론 거부를 다룬다. fence 후 동일 lease 갱신과 Host cohort/부분 Arm 거부도 검사한다.

별도 S 모의 Host, 실제 P writer/worker, 등록 단말 HTTPS에서 발급·수용 응답 유실·활성화/같은 key 회수를 확인한다. 활성화까지 native effect는0개다. 그 뒤 별도의 작업자 StartRun, 실제 Arm/prepare/authorize, native 성공 근거와 인계 관측, 자원 해제 및 Run 완료를 연결해 독립 file-device log의 effect1개를 확인한다. 실제 장비 동작이나 물리 인수를 의미하지 않는다.

전용 UI, 장기간/최대 크기/부하 검증, 만료하거나 미확정인 작업의 전체 운영 복구, physical qualification 자료·전문 심사·production trust 배포/갱신, 취소/rollback/업그레이드 복원과 전체 플랫폼 지원 범위는 미완료다. 첫 물리 셀은 NOT_COMMISSIONED다.

배포 범위: S runtime image는 현재 executor/process 도구와 관리 프로세스를 담는다. phase54 당시에는 Host crate만 build하고 별도 test-harness 서버를 사용했다. 후속 [제품 Host 서비스](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/HOST_SERVICE.md)는 rx-hostd를 image에 포함하고 실제 Linux clock으로 검증한다. physical factory/전체 배포 관리 연결은 후속이다.
