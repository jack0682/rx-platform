# 승인된 장비 작업의 연결 변경안

phase67. 현재 승인된 장비 패키지를 다시 검증하여 작업 연결 후보와 영향 범위를 저장한다. 상태는 PROPOSED 또는 IMPACT_REVIEWED이며 둘 다 실행 가능한 구성이 아니다. 활성 CellConfiguration, Work, Host binding, grant/permit, qualification과 Run을 수정하지 않는다.

## 이 단계가 필요한 이유

현재 CellConfiguration.steps는 장비 Intent/guard와 공정 실행 단계의 관계를 함께 담는다. compiled process가 있으면 해당 단계와 host/Intent가 정확히 맞아야 한다. 장비 작업만 교체하면 공정 원본/컴파일 결과·Host의 static allowed Intents가 어긋날 수 있다.

따라서 이 변경안은 **공정 순서를 갖지 않는 후보 binding**을 저장한다. 후보 StepBinding의 predecessors는 빈 배열이며 실제 실행 그래프라고 해석하지 않는다. 다음 단계에서 후보를 공정 작성/검토와 Host 재바인딩 절차에 결합해야 한다. 기존 process를 지우거나 임의의 순서로 대체하지 않는다.

## 명시적으로 선택하는 입력

Propose는 id/cell, 정확한 device review ID/report revision/review digest/decision revision, binding ID별 Selection과 reason을 받는다.

| Selection | 의미 |
|---|---|
| action | 승인된 catalog의 작업 이름 |
| host | 연결할 Host 이름. 새 Host이면 미해결 등록 항목을 기록 |
| conditions | catalog가 요구한 condition ID마다 명시적인 Condition 식 |
| completion_postconditions | native 성공에 추가할 현재 조건 |
| handover_max_age_ns | 인계 근거의 명시적 유효기간 |

Intent의 target/profile/site/calibration/resource/시간 제한을 입력으로 덮어쓰지 않는다. 승인된 catalog에서 정확한 Intent를 가져오고 결과표도 같은 원본에서 가져온다. 현재 step ID와 같으면 원래 step digest와 증가한 condition revision을 기록하며, 새 ID면 revision1인 후보가 된다. 이것은 active step revision을 갱신한 것이 아니다.

catalog의 필수 condition ID를 정확히 연결해야 하며 누락/추가·handover0·미지 action·빈 All/Any/뒤집힌 Range 등 잘못된 식을 거부한다. 빈 관측 문맥의 condition evaluator는 **구조 검사에만** 사용하며 실제 PASS를 주장하지 않는다.

현재 FactSpec에 없는 fact/schema/unit, 다른 site configuration, 미등록 Host, 관측할 수 없는 완료는 미해결 issue로 기록한다. 조건이 없다고 true를 채우거나 결과표가 없다고 성공 규칙을 만들지 않는다. native 결과표가 없으면 Unobservable 후보와 별도 완료/복구 검토 필요를 보존한다.

## 근거와 영향 범위

각 제안과 영향 검토 전에는 현재 device software approval·정확한 최신 보고서/서명·Store 원본과 정책을 worker에서 다시 검증한다. writer는 현재 역할·등록/authority·구성·boot/30초 ticket과 승인 revision을 다시 확인한다. 데이터 원본이 바뀐 승인이나 과거 승인 receipt만으로 새 계획을 만들지 않는다.

영향은 현재 셀의 모든 host/resource/scope에 후보의 host/resource를 합쳐 구한다. 관련 셀을 선택하고 그 셀의 공유 host/resource/scope까지 반복해서 포함한다. 최대64셀을 허용한다. 기존 process-change의 영향 계산은 추가 seed가 빈 경우로 유지한다.

계획에는 영향 셀의 당시 구성 digest, definition/envelope/recipe와 host/resource/scope 목록을 기록한다. 후보의 새 자원도 origin의 영향 목록에 포함한다. 관련 셀을 볼 권한이 없는 사용자는 제안을 commit하거나 영향 검토를 수행할 수 없다. 새 공유 셀이 생기거나 관련 구성이 바뀌면 이전 영향 검토 대상은 더 이상 현재 문맥과 일치하지 않는다.

## 저장·검토와 표시

before configuration은 기존 immutable configuration 저장소에 보관하고 Plan에는 참조를 둔다. 원래 선택 입력·candidate·원래 step digest·미해결 항목·승인/catalog/package 출처·builder digest·영향을 Definition으로 묶어 plan digest를 계산한다. 입력은128KiB, Plan과 before를 합친 조회 자료는768KiB 이하로 제한한다.

제안·history·event·request-key 결과를 원자 기록한다. 영향 검토는 제안자와 다른 Verifier이고 모든 영향 셀에 접근할 수 있어야 한다. 검토 직전에 원본을 다시 검증하고 후보/issue/영향을 재계산해 기존 Definition과 비교한다. 미해결 항목이 있어도 영향 검토 사실은 기록할 수 있으나 그것이 항목 해결이나 적용 허가를 뜻하지 않는다.

영향 검토 후 revision은 증가하지만 Definition/plan digest는 유지한다. 변경된 입력은 새 계획으로 제안한다. 동일 요청 회수는 원래 기록을 반환하고 현재성이 사라졌다고 과거 사실을 덮어쓰지 않는다.

Detail의 context_current는 현재 구성/builder/영향과의 일치, device_approval_current는 현재 등록된 장비 승인과의 일치다. 둘 다 파일을 방금 다시 검증했다는 proof는 아니다. activation_authorized/configuration_changed/application_supported는 현재 모두 false다.

## API

| 경로 | 기능 |
|---|---|
| POST /api/v1/device-binding-plans | 현재 승인과 원본을 다시 검증하고 제안 저장 |
| GET /api/v1/device-binding-plans?cell=…&after=… | 접근 가능한 요약50개와 다음 cursor |
| GET /api/v1/device-binding-plan?cell=…&id=… | before·후보·영향·미해결 항목·현재성 |
| POST /api/v1/device-binding-plan/impact-review | 정확한 revision/digest에 대한 독립 영향 검토 |

계획 편집/영향 검토 UI와 실제 적용 endpoint는 후속이다. 통합 시험은 실제 S JTC package/report/approval을 거친 P 개발 API에서 수행한다. 현재 동작과 시험 결과는 [phase67 기록](../../../references/implementation/phase67_checks.json)에 둔다.

## 다음 연결

1. 미해결 host/fact/site/completion 항목을 명시적으로 해결한 후보를 공정 작성의 선택지로 연결한다.
2. candidate와 기존 실행 순서를 함께 재컴파일·검토하고 source provenance를 보존한다.
3. Host binding 변경·quiet/fence·원장/세대·현재 장비 제어권을 검증한다.
4. 실제 적용은 APPLIED_UNQUALIFIED부터 시작하고 재검증/qualification·별도 사용자 시작을 요구한다.

이 단계가 위 네 절차를 완료한 것으로 세지 않는다. 원래 Work/증거는 원래 구성을 유지해야 한다. 실물 자원 alias·관측의 적합성, production JTC provider와 현장 인수는 여전히 미완료이며 첫 물리 셀은 NOT_COMMISSIONED다.

phase68에서 [후보의 공정 작성·컴파일 연결](DEVICE_PLAN_AUTHORING.md)을 추가했다. 현재 영향 검토가 끝났고 issue가 없는 plan만 선택하며, v2 입력에 plan/step/action 출처를 보관한다. 현행 process review의 active-step 검증만으로 이를 승인하지 않도록 명시적 거부 경계를 유지한다.
