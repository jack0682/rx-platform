# 승인 공정의 변경 계획·영향 검토·staging·적용 준비

2026-09-12. 이 구현은 SR03의 `PROPOSED → IMPACT_REVIEWED → STAGED`와 그 뒤의 적용 준비를 연결한다. **현재 설치 구성은 아직 바꾸지 않는다.** `APPLIED_UNQUALIFIED → QUALIFIED_ACTIVE`에는 Host 구성 ack, 잔류 작업/지지 처분, 이전 실행 참조 보존과 새 qualification을 연결해야 한다.

이는 승인 화면의 버튼을 실행 선택 pointer에 바로 연결하지 않기 위한 실제 변경 경로다. Stage까지는 운영 권한을 바꾸지 않으며, 등록 단말의 ReleaseManager가 적용 준비를 명시적으로 요청한 경우에만 영향 셀의 epoch/permit/fence를 처리한다.

## 승인 결과에서 대상 구성을 만드는 방법

제안에는 공정 검토 ID·검증 revision·review digest·승인 decision revision과 변경 이유를 고정한다. 패키지 worker가 원본 Store·정책·verifier authority·signed report·원문과 결과를 다시 검사해야 새 제안을 만들 수 있다. P의 최신 소프트웨어 승인도 그대로 유효해야 한다.

대상 구성은 임의의 새 CellConfiguration을 클라이언트에게 받아 저장하지 않는다. 검토한 configuration snapshot과 실제 검증된 resolved process로 만든다.

- 각 compiled operation node의 binding을 검토 시 선택한 기존 StepBinding에 연결한다.
- host, normalized intent, 실행 조건, completion 규칙, condition ID/revision, handover age 등 기존 작업 규칙을 복사한다.
- 새 StepBinding ID는 compiled node ID다. 같은 template을 여러 위치/반복에 쓰면 각각의 node ID로 분리하고 원래 template ID를 `step_origins`에 보존한다.
- 정적 predecessor 목록은 비운전 검토에서 원문 순서를 확인한 뒤 compiled tree가 담당하도록 전환한다. P의 기존 configuration validator가 요구하는 형태로 명시적으로 바꾼다. 서명된 원문/결과와 predecessor 순서의 독립 검사는 앞 검토 경로와 재검증 worker에서 실행한다.
- recipe 참조는 실제 resolved process의 SHA-256/schema/size로 바꾸고 process를 포함한다. 나머지 셀 구성 필드는 그대로 유지한다.
- before/after는 별도의 내용 hash 기반 불변 구성 문서로 보관한다. 구성이 같거나, 장비 operation이 없거나, 확장된 구성 record가 현재1MiB 한도를 넘으면 거부한다.

기존 definition/envelope를 새 qualification으로 승격하지 않는다. 이 단계에서 유지하는 참조는 기존 하드웨어/제약 문맥이며, 공정 순서 변경이 envelope·검증 범위에 주는 영향은 재검토 대상이다. 새 구성에 기존 qualification을 붙여 실행시키는 구현은 없다.

## 영향 범위와 검토

기본 단위는 셀 전체다. 변경 원점에서 시작해 다음 중 하나를 공유하는 셀을 고정점까지 포함한다.

1. scope
2. command/support resource
3. Host

Host만 같고 scope/resource 이름이 다르더라도 공통 프로세스·제어 경로의 영향을 놓치지 않도록 포함한다. 현재 closure 한도는64셀이다. 알 수 없는 의존성이 없다고 추정해 일부 node만 영향 없음으로 취급하지 않는다. 현재 builder는 장비/Host/layout을 추가하는 범용 변경기가 아니며, 그런 변경은 영향 그래프를 확장해야 한다.

각 영향 셀의 전체 configuration digest, definition/envelope/recipe 참조와 Host/scope/resource 목록을 고정한다. qualification·recovery 검토와 Host 구성 확인 필요성을 명시한다. 이 정적 영향 문맥과 실행 중인 Run/작업/자원/사건의 현재 blocker를 구별한다. 정상 작업 진행만으로 제안의 정적 digest가 바뀌지는 않는다.

모든 영향 셀에 현재 접근권이 있어야 제안·조회·검토·staging·준비를 진행할 수 있다. origin 하나의 권한으로 다른 셀의 변경 정보를 얻거나 권한을 철회하지 못한다.

변경 제안자와 다른 Verifier 계정이 변경 이유와 before/after·scope·검증/복구 영향을 검토한다. 계획 digest와 revision을 명시하고 의견을 기록한다. 영향 검토를 바꿀 때도 CAS와 이력을 유지한다. Stage는 ReleaseManager가 요청하며, package/report/현재 승인과 target 재생성을 다시 확인한다.

## 적용 준비의 효과

등록된 현재 단말에서 ReleaseManager가 `BeginPreparation`을 명시적으로 요청한다. 이 API는 실제 적용 완료를 뜻하지 않는다.

하나의 transaction에서 다음을 수행한다.

- 중첩된 영향 범위에서 다른 변경의 준비가 진행 중이면 거부한다.
- 현재 검토/승인, 계획 digest/revision, builder identity와 모든 영향 구성 digest를 다시 확인한다.
- 영향 셀 전체에 새 epoch와 scope epoch를 기록하고 `CONFIGURATION_CHANGE` latched block을 추가한다.
- 기존 미진입 permit/queue와 live mandate를 기존 invalidation 규칙으로 봉인한다. 실제 진입 가능성을 배제하지 못한 작업은 UNKNOWN/미해결 상태와 자원을 보존한다.
- Run·start attempt·native disposition도 기존 invalidation 규칙으로 처리한다. 기존 결론이나 증거를 새 성공으로 바꾸지 않는다.
- 관련 Host에 fence를 outbox로 넣고 각 메시지 ID/목표 epoch/scope를 Preparation에 묶는다. Preparation, 변경 revision/history/event와 request 결과를 함께 기록한다.

같은 request key/body는 최초 준비 결과를 회수한다. 응답 유실을 이유로 새 epoch/fence를 중복 발행하지 않는다. Runtime 재시작이나 다른 hold로 preparation의 boot/epoch가 낡아지면 조회에 stale을 표시한다. `refresh=true`와 최신 변경 revision을 명시해야 다시 준비한다. 이전 준비 이력·기존 block·작업/자원은 지우지 않는다.

현재 준비 중인 변경을 자동 취소하거나 이전 운전 권한을 복구하는 API는 없다. 조정/취소도 남은 물리 상태와 적절한 복구 절차를 요구한다.

## 조회와 미충족 조건

조회는 stored before/after와 현재 blocker를 반환한다. 최대256개 항목과 전체 개수/잘림 여부를 구분한다.

- 현재 configuration/승인 또는 준비 boot/epoch가 달라짐
- 아직 처분되지 않은 Run
- NONE/UNRESOLVED 또는 disputed 작업
- 보유/격리된 resource
- 열린 사건
- 준비 메시지에 대한 Host fence 미확인
- Host 구성 acknowledgement 필요

fence 확인은 해당 준비의 정확한 메시지·epoch·scope와 현재 등록된 Host boot/journal을 대조한다. **fence 확인을 구성 적용 ack로 대신하지 않는다.** Host의 공정 문맥 서비스/receipt와 P transport client는 [Host 계약](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/PROCESS_CONFIGURATION.md)에 구현했다. [P 영속 coordinator](HOST_CONFIGURATION_DISPATCH.md)가 명시적 Batch 요청·자동 전송·receipt 반영을 연결한다. 현재 준비에 맞는 Host 확인이 없는 동안 `HOST_CONFIGURATION_ACKNOWLEDGEMENT_REQUIRED`는 남는다. Host 확인이 부족한 상태에서 after를 설치하거나 `APPLIED_UNQUALIFIED`/`QUALIFIED_ACTIVE`를 기록하지 않는다.

STAGED까지는 `applied=false`, `activation_authorized=false`다. [P 적용](PROCESS_APPLY.md) 후에는 APPLIED_UNQUALIFIED와 `applied=true`를 기록하지만 운전 허가는 false다. 나머지 blocker가 적다고 물리적 정지·지지 처분이 확인됐다는 뜻도 아니다. Host별 receipt의 적용/미적용/미확인과 현재 준비와의 일치 여부를 별도로 표시한다. 모든 Host 확인 후에도 P 적용과 qualification은 후속 단계다.

## API

기존 browser BFF의 `{request_key, command}`를 사용한다. 아직 변경 전용 화면은 제공하지 않는다.

| API | 역할·내용 |
|---|---|
| POST `/api/v1/process-changes` | Engineer: 검토/승인 참조와 이유로 변경 제안 |
| POST `/api/v1/process-change/impact-review` | 별도 Verifier: target(change/cell/expected/plan_digest)와 의견 |
| POST `/api/v1/process-change/stage` | ReleaseManager: 검토한 target을 재검증하여 STAGED |
| POST `/api/v1/process-change/prepare` | 현재 등록 단말의 ReleaseManager: target과 refresh 여부로 적용 준비 |
| POST `/api/v1/process-change/configure-hosts` | 현재 등록 단말의 ReleaseManager: 영속 Host 요청 Batch를 명시적으로 승인 |
| POST `/api/v1/process-change/apply` | 현재 등록 단말 ReleaseManager: fresh Host 확인·패키지 재검증 후 P 구성을 APPLIED_UNQUALIFIED로 교체 |
| GET `/api/v1/process-change?cell=...&id=...` | Engineer/Verifier/ReleaseManager: 모든 영향 셀의 권한 확인 후 before/after·blocker 조회 |

제안·stage worker ticket은 현재 boot/등록된 Store owner/정책에 결합하며30초만 유효하다. 현재 역할과 영향 셀 권한은 cached 결과 회수 전에도 확인한다. 같은 key의 다른 의도, 오래된 revision/plan digest, 회수된 소프트웨어 승인은 거부한다.

CONFIGURATION_CHANGE는 내부 block reason이며 기존 wire CellReason의 EXTERNAL_RESTRICTION으로 투영한다. UI에도 ‘구성 변경 준비’로 표시한다. frozen base/cell 규범 파일과 enum 값을 바꾸지 않았다.

## 다음 연결

1. 적용 후 새 epoch의 fence 확인과 Host 재검증 연결.
2. 실제 미적용/적용/불명, 혼합 구성을 구별한 재조정 및 중단/취소 정책.
3. 적용된 구성의 취소/복원에서 Run/증거의 불변 구성 참조를 유지.
4. APPLIED_UNQUALIFIED의 old qualification 보관/철회와 새 qualification 절차.
5. 변경 검토·준비·진행·미충족 조건 UI.

위 작업은 이번 staging/준비의 완료로 간주하지 않는다. 실장비는 아직 NOT_COMMISSIONED다.
