# 장비 변경 후보를 공정 작성·컴파일에 연결

phase68. 현재 영향 검토가 끝난 장비 binding plan을 공정 초안의 선택지에 포함하고, 선택 출처를 컴파일 입력과 패키지에 보존한다. active CellConfiguration/Host binding/Work/qualification/Run은 변경하지 않는다.

## 선택 가능한 후보

기존 GET binding-options는 active steps의 기존 catalog를 그대로 반환한다. 후보를 포함하려면 같은 경로에 POST로 `{cell, device_plans:[{id,revision,plan_digest}]}`를 보낸다. 이는 조회이며 설정을 저장하지 않는다.

P는 각 plan의 정확한 revision/digest, IMPACT_REVIEWED 상태, 미해결 issue 없음, 현재 builder/구성/영향·device approval과 전체 영향 셀 접근권을 검사한다. 최대16 plan을 허용하고 중복 plan ID를 거부한다. 서로 다른 plan이 같은 binding ID를 제공하면 순서로 선택하지 않고 충돌로 거부한다. 합친 후보는 최대512개다.

동일 binding ID의 active step은 선택한 plan의 후보로 표시할 수 있지만 active 값 자체를 덮어쓰지 않는다. 응답에는 source plan, required_cells와 별도의 composite catalog digest가 있다. plan이 없는 기존 catalog의 digest 계산은 유지한다.

## 공정 초안 저장·회수

기존 Save에 선택한 device_plans를 명시하고 기존 alias→binding ID selections를 사용한다. P는 같은 composite catalog를 재계산하여 exact Intent/Host·step digest를 보관한다. 선택한 모든 plan이 실제 선택에 사용되어야 하며, 의미 없는 출처를 추가하지 않는다.

Binding Version은 plan refs/필요 셀/alias별 device_sources를 보관한다. 같은 key의 결과 회수와 과거 binding 조회도 현재 영향 셀 접근권을 확인한다. 선택한 plan이나 device approval이 더 이상 현재가 아니면 DEVICE_PLAN_CHANGED를 반환하며 원래 snapshot은 보존한다. 예전 active step으로 조용히 대체하지 않는다.

내보내기는 현재 source/binding revision·구조·catalog·plan·권한을 다시 검사하고 선택값, 원래 step digest, 실제 ActionBinding, provenance를 재계산해 저장본과 대조한다. 이 단계는 registry/승인 metadata의 현재성을 검사하며 원본 파일을 새로 취득한 execution proof가 아니다. 후속 승인/적용은 실제 원본 재검증을 요구한다.

## 컴파일 입력 v1/v2

변경 후보를 쓰지 않은 입력은 기존 `rx.process-compile-input.v1`과 hash를 유지한다. 후보를 쓰면 v2이며 alias마다 다음을 포함한다.

- exact plan ID/revision/digest
- plan 안의 binding ID
- 전체 candidate step digest: 조건/인계 정책의 원래 출처 참조
- 실제 Host/Intent의 action digest

v2 bindings_digest는 bindings와 device_sources를 함께 포함한다. v1에 출처를 끼우거나 v2에서 출처를 삭제·변경하거나 Host/Intent를 바꾸면 integrity 검사를 통과하지 못한다. plan은 최대16개이고 provenance alias는 실제 binding에 존재해야 한다.

S compiler는 실제 새 Intent로 ResolvedProcess와 BT XML을 생성하고 compile-report에도 provenance를 유지한다. process package의 signed authoring/compile-input.json에도 전체 v2 입력을 보존한다. 이 참조 자체가 S에 실행 권한이나 P의 plan registry 접근권을 주지는 않는다.

## 현행 검토와 다음 연결

현재 P process review는 active configuration의 step/guard를 검증한다. 장비 plan 출처를 소비하는 새 검토 경로가 아직 없으므로 v2 device_sources가 있는 입력은 명시적 platform issue로 승인하지 않는다. 기존 step과 Host/Intent가 같아도 출처만 제거하고 새 guard가 적용된 것으로 취급해서는 안 된다.

다음에는 process review request/job에 exact plan refs와 후보 guard/configuration 문맥을 결합하고, 모든 device package/approval/impact를 현재 원본으로 재검증해야 한다. 그 뒤 Host native/static binding 변경을 증명하는 경계와 APPLIED_UNQUALIFIED 적용·qualification을 이어야 한다. 이번에 이 검토/적용을 완료한 것으로 세지 않는다.

기존 UI는 API로 저장한 후보 binding과 v2 출처를 읽고 내려받을 수 있도록 decoder를 확장했다. 미해결/오래된 plan은 조작을 차단하고 이전 snapshot을 표시한다. 새로운 plan을 고르는 전용 UI는 후속이다.

시험은 정확한 v2 출처·원자 저장/응답 유실·plan 갱신 후 export 거부, signed package/recompile에서의 출처 보존·변조 거부, legacy review의 명시적 거부를 다룬다. 실제 JTC package/report/impact review→P draft API→S compiler/unsigned package 연결을 실행했다. [phase68 증거](../../../references/implementation/phase68_checks.json)를 따른다. 첫 물리 셀은 NOT_COMMISSIONED다.
