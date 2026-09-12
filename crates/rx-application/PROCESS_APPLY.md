# 활성 공정 구성 교체와 작업별 구성 이력

상태: phase51 구현 초안. [변경 계획](PROCESS_CHANGE.md)의 STAGED에서 APPLIED_UNQUALIFIED까지 연결한다. [Host 전송](HOST_CONFIGURATION_DISPATCH.md)의 receipt를 바탕으로 **P가 선택하는 공정 구성**을 교체한다. 로봇/PLC/native driver의 설정을 바꾸거나 실제 운전을 시작하는 기능은 아니다.

## 적용 대상과 현재 한계

검토·승인한 공정 패키지에서 생성한 새 recipe/process/step 구성이 대상이다. 기존 단계의 intent·Host·조건·완료/인계 규칙을 보존하는 builder만 사용한다. 임의의 장비 definition/envelope/profile/통신 설정을 API로 받지 않는다. 영향은 공유 Host/resource/scope closure 전체이며, 공정 recipe가 바뀌지 않는 이웃 셀도 자격 재검토와 epoch/fence 경계에 포함한다.

상태 흐름은 PROPOSED → IMPACT_REVIEWED → STAGED → APPLIED_UNQUALIFIED다. [재검증 근거·독립 검토](REQUALIFICATION_REVIEW.md)를 연결했다. [QUALIFIED_ACTIVE와 별도 시작 연결](QUALIFICATION_ACTIVATION.md)을 추가했다. 변경 취소/복원과 전체 물리 검증은 후속이다. 기존 변경 block을 적용 성공으로 해제하지 않는다.

## 적용 절차

`POST /api/v1/process-change/apply`는 기존 `{request_key, command}`와 Transition의 change/cell/expected/plan_digest를 받는다.

1. 현재 등록 단말의 ReleaseManager와 전체 영향 셀의 사용자·단말 교집합 권한을 검사한다.
2. 정확한 STAGED revision/plan, 현재 검토 승인·구성·preparation/P boot를 검사한다.
3. 모든 영향 Run의 COMPLETED/ABANDONED, 미확정/분쟁 work 없음, 보유/격리 resource 없음, 관련 열린 사건 없음과 준비 fence ack를 검사한다.
4. 모든 Host의 같은 Task/preparation/request에 대해 APPLIED_UNQUALIFIED receipt와 현재 boot/journal/binding/cohort/block 문맥을 확인한다.
5. package worker가 writer 밖에서 실제 Store bytes·현재 policy·서명·검토 자료를 다시 검증한다. PASS Boolean이나 browser 제공 after configuration을 받지 않는다.
6. commit 직전에 1–4, ticket의 boot/등록/시간, 검토 자료·target 동일성을 다시 검사한다.
7. 구성/이력/epoch/fence/자격/변경 결과를 한 transaction으로 기록한다.

검증 도중 역할·단말·Host 문맥·구성·준비·정책이 바뀌면 새 적용을 거부한다. commit 이후 응답만 잃었으면 같은 key/body가 기존 적용 기록과 fence ID를 회수한다. 새 key 또는 새 revision으로 효과를 재수행하지 않는다. 현재 접근권 검사는 cached 결과 회수에도 적용한다.

## Host 최근 확인의 소유권

영속 Task의 과거 observation만으로 적용하지 않는다. configuration worker가 조회/Apply 호출 **직전** P 시각을 얻어 응답과 함께 writer에 전달한다. writer는 조회 시작부터 3초 이내인 응답만 현재-process read proof로 인정한다.

증명은 Task ID → `(read_started, observation digest)`의 Engine 내부 메모리에 있다. 영속 receipt는 기존 원장에 남지만, 최근 읽었다는 자격은 재시작 후 복원하지 않는다. 반복한 동일 observation의 freshness 갱신 때문에 같은 Task 사건을 계속 DB에 쓰지 않는다. 오래된 proof는 새 proof를 기록할 때 제거한다.

적용 preflight와 commit은 현재 영속 observation의 digest가 그 proof와 같고 현재 P clock에서 3초 이내인지 확인한다. RPC 오류·Host 세대 변화·분쟁/현재 조건 변화는 별도 현재 상태 검사로 막는다. ApplicationRecord에는 적용 시 사용한 Host별 request/receipt/observation digest와 read 시작 시각을 증거로 남긴다.

3초는 이 초안의 **비운전 구성 선택**에 대한 응답 age 한도다. 지속 물리 안정성이나 운전 허가의 유효시간이 아니다. Host의 실제 adapter quiescence 검사는 앞선 Host 문맥 기록 시점에 수행됐으며, 그 뒤 물리 상태가 계속 유지됐다는 보증으로 확대하지 않는다. 적용 후 새 qualification/현장 검증이 필요하다.

## 하나의 transaction에 들어가는 것

| 대상 | 적용 결과 |
|---|---|
| 과거 Run | 빠진 최초 구성 binding을 검증해 보관. 기존 binding/결과/예산/slot/증거를 새 구성으로 덮어쓰지 않음 |
| origin CellConfiguration | 검토된 after configuration으로 교체 |
| 이웃 셀 구성 | recipe는 유지하고 전체 영향 경계의 새 epoch/scope를 기록 |
| Qualification | active 자격을 제거하고 기존 자격을 history 및 ApplicationRecord에 보관 |
| mode/commissioning | SETUP. 기존 NOT_COMMISSIONED는 유지, 그 외는 REVALIDATION_REQUIRED |
| Authority/block | epoch·모든 scope 증가, 옛 authority 봉인, 새 latched CONFIGURATION_CHANGE와 fence outbox |
| 구성 선택 이력 | 셀별 active selection 및 `(change, cell)` 불변 history, before/after artifact |
| 변경 결과 | APPLIED_UNQUALIFIED revision/history/event와 동일 key 결과 |

준비 단계에서 증가한 epoch를 실제 선택 교체 시 한 번 더 증가시킨다. Host receipt는 **교체 직전 준비 epoch**에 대한 증거로 보관하며, 새 epoch의 fence ack로 바꿔 해석하지 않는다. 적용 직후 새 fence는 outbox에 있고 기존 dispatcher가 전달한다. 새 epoch 확인과 재검증 없이 운전을 허용하지 않는다.

저장 실패는 이 모든 변경을 rollback한다. 네트워크/Host 적용은 P DB와 분산 transaction이 아니다. Host 문맥은 이미 기록됐으나 P 적용은 실패한 상태가 가능하고, 같은 준비/요청의 재조회와 같은 적용 key 회수가 그 상태를 다룬다.

## 작업별 불변 구성

새 Run 생성은 `runconfiguration/<Run ID>` binding과 내용 hash 기반 CellConfiguration artifact를 Run·요청 결과와 함께 저장한다. binding은 Run/cell/configuration artifact를 연결하며 recipe/envelope도 Run과 대조한다.

기존 초안 DB에는 binding 없는 Run이 있을 수 있다. 해당 셀에 구성 교체 이력이 한 번도 없고 Run의 recipe/envelope가 초기 현재 구성과 일치할 때만 최초 구성을 읽거나 binding을 보완한다. 첫 교체 transaction은 해당 셀의 모든 기존 Run에 필요한 binding을 먼저 만든다. 이미 구성 선택 이력이 있는데 binding이 없으면 추정하지 않고 무결성 오류로 거부한다.

구성 이력의 읽기 호환과 미완료 변경 계획의 실행 호환은 별개다. builder identity가 다른 옛 STAGED 계획을 새 코드에서 자동 채택하지 않는다. 미완료 Host 효과가 남은 업그레이드의 이행/복원 절차는 아직 구현하지 않았다.

`execution_snapshot`, `executor_artifact`, `production_view`는 Run이 소유한 구성을 읽는다. snapshot의 cell revision/epoch/scope·접근권은 현재 셀에서, resolved/process progress는 해당 Run의 불변 구성에서 가져온다. 과거 완료 Run 조회에 현재 recipe를 끼워 넣지 않는다. 새 요청 허용 여부는 현재 상태에서 따로 검사하며, 시작/실행 admission은 Run 구성 전체가 현재 셀 구성과 일치해야 한다.

이번 공정 변경은 definition/envelope 교체를 지원하지 않으므로 frozen 협상 의미를 바꾸지 않는다. 미래에 셀 definition 자체를 교체하려면 과거 definition에 대한 접근·협상 정책을 별도로 설계해야 한다.

## 조회 의미

`Detail.applied=true`는 이 변경의 P 적용이 기록됐다는 역사 사실이다. 이후 다른 구성을 선택한 경우 current 구성과의 차이는 CONTEXT_CHANGED로 표시한다. 운전 허가는 항상 false다. APPLIED_UNQUALIFIED 조회에는 REQUALIFICATION_REQUIRED를 표시하고 과거 preparation proof와 적용 후 epoch/fence를 구별한다.

`host_configuration.platform_configuration_applied`도 P 적용 이력 여부다. `all_hosts_acknowledged`는 기존 준비 문맥에 대한 값이므로 적용 후 새 epoch에서 운전 가능 표시로 사용하지 않는다. 변경 전용 UI와 재검증 진행/결과 표시는 아직 없다.

## 검증

[phase51 기록](../../../references/implementation/phase51_checks.json)에 범위별 결과를 보관한다. 원자성/응답 유실, 현재 proof 필요와 commit 중 통신 문제, 일부 Host 확인과 단말 회수, 실제 완료/인계된 과거 공정의 artifact·작업/생산 조회 및 새 Run binding을 검사한다. 별도 S 모의 Host와 실제 P writer의 mTLS 경로는 Host 응답 유실 회수 후 P 적용과 재시작 보존까지 검사한다. test-only 서명자와 모의 장비의 증거이며 실물 commissioning을 대신하지 않는다.
