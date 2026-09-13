# 적용 후 재검증 요청·근거·독립 검토

상태: phase52 구현 초안. [P 구성 적용](PROCESS_APPLY.md) 후 검증 자료를 고정하고 실제 바이트·서명·현재 문맥을 확인하여 독립 검토 결정을 기록한다. 승인 scope는 `REQUALIFICATION_EVIDENCE_REVIEW`다. Qualification/QUALIFIED_ACTIVE, Host 차단 해제와 Run 시작은 생성하지 않는다.

## 책임과 신뢰 경계

정책 소유자는 정확한 구성별 acceptance plan·필수 항목·의존 자료·제약과 허용 검증자 공개키를 제공한다. 등록 단말의 ReleaseManager가 새 epoch/fence로 검증 범위를 고정한다. 외부 검증자가 항목별 결과와 근거를 만들고 서명하며, Engineer가 반입한다. 요청자 및 반입자와 다른 Verifier가 이를 다시 검토한다. 이후 자격 활성화 경로는 이 승인과 현재 Host/장비 문맥을 다시 연결해야 한다.

서명은 허용된 검증자가 한 주장이다. 서명 검사나 여섯 영역의 존재만으로 시험의 기술적 충분성·측정의 정직성을 증명하지 않는다. 구체 시험·위험·기대 결과·허용 조합은 검토된 acceptance plan/criterion specification의 책임이다. 신호 GOOD나 사람 체크 표시로 실제 보호 기능 검증을 대체하지 않는다.

## 정책·필수 범위

정책 v1/v2는 startup의 pinned JSON이다. v2의 목적 범위와 실제 발급/활성화는 [자격 활성화](QUALIFICATION_ACTIVATION.md)를 따른다. `package_intake.qualification_policy: {path, sha256}`로 구성한다. 공개 API에서 정책/키를 설치하지 않는다. startup 및 각 worker 작업에서 파일 pin을 검사하며 hot reload하지 않는다.

| 객체 | 필드·책임 |
|---|---|
| Profile | cell, 정확한 전체 configuration ArtifactRef, definition/envelope, environment, acceptance_plan, limitations, dependencies, criteria |
| Criterion | 고유 ID, area, 불변 specification ArtifactRef, 허용 evidence_schema |
| Key | key ID/public_key, 허용 validator digest 집합, SIMULATION/PHYSICAL 허용 environment 집합 |

각 profile은 SOFTWARE, EQUIPMENT, CELL_INTEGRATION, RECOVERY, PROTECTION, OPERATIONS 여섯 영역을 모두 명시한다. 같은 영역에 여러 항목을 둘 수 있고 모든 항목이 필수다. 적용 범위상 별도 보호 동작이 필요 없다는 판단도 specification과 근거의 검토 대상이며 필수 영역을 삭제하지 않는다.

최대값의 곱집합을 추론하지 않고 정확한 전체 구성 hash를 가리킨다. 셀의 definition/envelope/recipe/site 설정과 단계별 profile/site/calibration 및 trajectory/tool/program/parameter/mode-transition/stream profile hash가 자료 집합에 포함돼야 한다. configuration/definition/envelope 자료도 실제 파일로 검증한다. placeholder hash에 맞는 원본이 없으면 보고서 검증을 통과하지 못한다.

정책 의미 digest는 키/profile/criteria/의존 자료의 정렬을 정규화한다. 파일 pin은 원본 bytes의 hash다. 의미가 같더라도 파일이 바뀌면 현재 worker는 거부한다. 실제 정책·키 갱신/배포와 활성 자격의 철회 연계는 후속이다.

## 요청과 세대

Begin은 새 request ID, 적용된 change ID/origin cell, 정확한 change revision, 영향 셀 전체의 expected_cells revision map, 등록 policy digest를 받는다. 누락·추가 셀을 거부한다. 현재 단말/사용자 교집합이 모든 영향 셀을 포함해야 한다. 변경은 APPLIED_UNQUALIFIED이고 선택된 구성이 적용 기록과 일치해야 하며, 활성 qualification을 덮어쓰지 않는다.

영향 셀 전체를 invalidate하여 epoch/scope·latched block을 갱신하고 fence outbox·Request/Job·사용자 요청 결과를 한 transaction으로 기록한다. 네트워크 대기는 하지 않는다. 같은 key/body는 같은 요청/fence를 회수한다. 새 요청은 세대를 바꾸므로 이전 검토의 현재성을 잃게 한다.

Request에는 전체 profile, 적용 기록 digest, Runtime boot, 각 셀 revision/epoch/scope/block 및 정확한 fence ID가 고정된다. 재시작 후 이전 요청의 현재성을 복원하지 않는다. 적용된 구성이 유지된다면 현재 단말과 revision으로 새 재검증 요청을 만들 수 있다.

## 보고서와 원본

```text
<import_root>/<directory>/
  qualification.json
  qualification.sig.json
  artifacts/<sha256>.bin
```

`rx.requalification-report.v1`는 원래 Request 전체, validator digest, `(cell, criterion)`별 checks를 담는다. 각 결과는 PASS/FAIL/NOT_RUN, 설명, evidence ArtifactRef 배열이다. 누락·추가·중복 항목을 거부한다. FAIL도 근거를 요구한다. 미수행은 NOT_RUN으로 명시하며 PASS만 남겨 나머지 시험을 숨기지 않는다.

서명 메시지는 다음 bytes다. 마지막 newline은 없다.

```text
RX-REQUALIFICATION-REPORT-SIGNATURE-v1\n<key ID>\n<report digest>
```

report digest는 `RX-REQUALIFICATION-REPORT-v1` canonical digest다. 패키지/공정 검토 서명을 재사용하지 않는다. 원래 요청, 현재 policy, signer/validator/environment 및 profile 전체를 대조한다.

worker는 요구한 문서/의존 자료와 모든 결과 근거를 실제로 읽고 hash/size/schema 참조를 확인한다. 자료 내부의 물리 성능·단위·시험 설계에 대한 전문 해석은 범용 parser로 구현하지 않았다. 이 단계가 검사하는 것은 서명된 정확한 자료 집합이다.

한 파일 최대8 MiB, 고유 원본 합계32 MiB다. report/signature/policy metadata는 각각 기존1 MiB 한도를 적용한다. PackagePath/capability 기반 읽기로 경로 이탈·symlink를 거부한다. worker당 동시 filesystem/crypto 작업은1개이고 writer는 파일을 직접 열지 않는다.

## 원자 저장·독립 검토

원본을256 KiB 조각의 base64 record와 전체 blob manifest로 저장한다. 조회 시 조각 hash와 전체 hash/size를 다시 대조한다. DB 문서1 MiB 한도에 맞춘 분할이며 transaction을 나누지는 않는다. 원본 조각·manifest·report version/history/event·요청 결과가 함께 commit된다. 기존과 충돌하는 조각은 덮어쓰지 않는다.

Version digest는 report/signature/기록자/시각/revision/검사 결과를 묶는다. 모든 결과가 PASS여야 `ready_for_review=true`다. FAIL/NOT_RUN 자료도 보관하지만 승인 준비로 표시하지 않는다.

승인 조건은 다음과 같다.

1. 현재 Verifier와 전체 영향 셀 접근권. 요청자 및 반입자와 다른 계정.
2. 최신 report revision/digest와 정확한 이전 decision revision.
3. 현재 policy/Runtime boot/적용 기록 및 셀 revision·epoch·scope·block 일치.
4. 모든 준비 fence ID/epoch/scope와 현재 Host boot/journal에 대한 ack.
5. 저장 원본과 현재 pinned policy를 worker에서 재검증한 private PreparedDecision.
6. commit 시 역할·문맥·version/CAS 재검사.

검토 중 Hold·재시작·구성/권한/정책 변경이 생기면 새 승인을 만들지 않는다. 응답 유실에는 같은 key로 원래 결정을 회수한다. 새 보고서 version을 반입하면 이전 승인은 새 자료의 승인이 아니다. 반려는 새 허용 권한이 없으므로 현재 대상/역할/CAS를 확인해 기록하며 crypto worker를 다시 요구하지 않는다.

## API와 표시

기존 인증·CSRF·terminal BFF와 `{request_key, command}`를 재사용한다.

| 경로 | 역할·내용 |
|---|---|
| POST `/api/v1/qualification-reviews` | 등록 단말 ReleaseManager: Begin 및 새 제한/fence |
| GET `/api/v1/qualification-review?cell=...&id=...` | Engineer/Verifier/ReleaseManager: Job/latest Version/Decision/현재성 |
| POST `/api/v1/qualification-review/reports` | Engineer: report digest/relative directory/expected version으로 반입 |
| POST `/api/v1/qualification-review/decisions` | 독립 Verifier: 정확한 version digest 승인/반려 |
| POST `/api/v1/qualification-review/artifact` | 읽기 전용 `{review,cell,reference}`. 해당 검토의 역사 version이 참조한 원본만 octet-stream 반환 |

GET은 context_current, fences_confirmed, approval_current, activation_authorized=false를 분리한다. 현재 policy는 Runtime에 등록된 의미를 말하며 파일을 상시 감시한다는 뜻은 아니다. worker는 신규 반입/승인마다 원본 policy pin을 다시 확인한다. 원본 조회에도 현재 권한과 모든 영향 셀 범위를 적용한다. 임의 경로나 일반 blob URL을 받지 않는다. 목록/과거 version 선택/검토 전용 UI와 정책 준비 도구는 후속이다. 현재 JSON 모델은 application 소유의 v1 artifact이며 base/cell 규범이나 선택 gRPC binding을 바꾸지 않았다.

## 다음 자격 활성화

승인 digest를 새 qualification ID/revision·configuration·현재 epoch에 묶고, 각 Host가 이를 수용한 durable receipt를 수집해야 한다. 부분 수용/불명·재시작을 조정한 뒤 P의 QUALIFIED_ACTIVE를 기록한다. 그때도 Run 시작은 별도 사용자 의도·현재 조건·Host 준비·mandate/permit 절차다.

검토 경로 자체는 activation authority가 아니다. [별도 발급·활성화](QUALIFICATION_ACTIVATION.md)는 v2 정책·원본 재검증·Host 수용을 요구한다. test fixture의 키·서명자·자료는 simulation protocol 검증용이며 기본 제품 trust나 현장 근거로 설치하지 않는다. 첫 셀은 NOT_COMMISSIONED다.

## 검증과 한계

[phase52 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase52_checks.json)을 따른다. 원장 시험은 준비/반입 원자성·응답 유실, 누락/위조/환경 불일치, NOT_RUN, 독립 검토/fence/변경 경합, version 교체와2 MiB 원본 분할/rollback/회수를 다룬다. 실제 P writer + 별도 S 모의 Host는 폴더 worker, 파일 손상/정책 변경 거부, 승인 후 Arm 거부, 재시작 후 승인 현재성 상실을 검사한다.

실제 loopback HTTP 서버에서 보고서 반입·독립 승인·원본/상태 조회를 추가 검증했다. 단말 신원이 없는 Begin 요청은 거부했다. Begin의 양성 경로는 실제 writer의 등록 단말 identity로 검사했으며 신규 경로의 전용 terminal HTTPS/browser UI 시험은 후속이다. 여섯 영역의 실제 장비·공정·보호 시험을 수행했다고 주장하지 않는다.

32 MiB 반입의 writer 점유/다른 셀에 대한 영향, 장기 이력 규모와 여러 동시 사용자 부하는 아직 측정하지 않았다. 용량 상한을 실제 운영 성능 보증으로 해석하지 않는다.
