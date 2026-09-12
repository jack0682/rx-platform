# 공정 초안의 장비 작업 바인딩

2026-09-11. 공정 source의 `Operation.binding` 이름을 현재 셀에 등록된 StepBinding에 연결하는 작성 경로다. 바인딩 저장이나 내보내기는 CellConfiguration, qualification, run, dispatch permit, native 실행을 변경하지 않는다.

## 선택 근거와 버전

작업 후보는 임의 코드·endpoint 입력이 아니라 현재 셀의 StepBinding에서 가져온다. Engineer/Verifier에게 step ID, Host, target, kind, resource와 intent/step digest를 보여준다. 이 후보 목록의 digest는 cell/definition/envelope/site config와 전체 step 정의에 묶인다. 따라서 intent뿐 아니라 조건·완료 규칙이 바뀌어도 이전 선택 기준과 구별된다.

공정 원문과 바인딩은 별도 revision이다. 바인딩 version은 source revision/content digest, 후보 catalog digest, 사용자가 고른 binding→step, 원본 step digest와 정규화한 ActionBinding을 보존한다. 동일 이름의 현재 step을 매번 다시 해석해 과거 선택을 바꾸지 않는다.

저장에는 현재 source revision, 현재 binding revision(CAS), 선택 당시 catalog digest가 필요하다. 원문의 구조 검사가 완료돼야 하며 source에 없는 binding 이름이나 목록에 없는 step은 거부한다. 일부만 선택한 상태는 missing 목록과 함께 저장할 수 있다. `complete`는 필요한 이름의 선택이 끝났다는 뜻이다.

제목만 달라져 source content digest가 같으면 기존 연결을 불필요하게 무효화하지 않는다. 원문 내용이나 catalog가 바뀌면 이전 version을 보존하되 SOURCE_CHANGED/CATALOG_CHANGED로 표시한다. 다시 저장할 때는 실제 현재 source revision과 catalog를 확인한다.

## 원자성·권한

최신 version, immutable history, 감사 사건과 원래 요청 결과를 한 transaction으로 저장한다. 같은 key/body는 원래 바인딩 version을 회수한다. 현재 Engineer 역할·셀 범위를 cache보다 먼저 확인하며, 다른 내용·오래된 CAS는 덮어쓰지 않는다. 저장 응답을 회수했다는 사실과 그 바인딩이 지금도 현재 구성이라는 판단은 별도다.

새로 저장하는 바인딩의 최대 선택 수는 128, 저장 snapshot은 256 KiB로 제한한다. source/cell과 결합한 draft ID를 검사한다. Verifier는 조회만 가능하다.

## API

| 요청 | 목적 |
|---|---|
| GET `/api/v1/process-draft/binding-options?cell=...` | 현재 등록 작업의 후보와 catalog digest |
| POST `/api/v1/process-draft-bindings` | `request_key` + `command{draft,cell,source_revision,expected,catalog_digest,selections}` 저장 |
| GET `/api/v1/process-draft-bindings?cell=...&id=...&revision=...` | 최신 또는 과거 binding과 현재성 판단. revision 생략 시 최신 |
| GET `/api/v1/process-draft-compile-input?cell=...&id=...&source_revision=...&binding_revision=...` | 현재 source와 완료된 current binding을 하나의 cut에서 내보냄 |

내보내기는 두 revision을 정확히 확인하고 내용/catalog가 현재와 다르거나 선택이 미완성이면 거부한다. source와 bindings를 따로 조회해 섞지 않는다.

## 하나의 컴파일 입력

`rx.process-compile-input.v1`에는 draft/cell, source/binding revision, source document digest, bindings digest, catalog digest와 실제 source/bindings가 들어간다. 내용 digest를 검사하지만 자체 digest는 서명이나 package 승인이 아니다.

S compiler는 기존 두 파일 입력을 유지하고 다음 입력을 추가한다.

```text
rx-process-compile --bundle process-compile-input.json NEW_OUTPUT_DIRECTORY
```

bundle digest·형식을 확인한 뒤 기존 source/intent 정규화·전개·병렬 resource 충돌 검사를 수행한다. 결과에 authoring provenance를 남기며 상태는 `COMPILED_NOT_QUALIFIED`다. 프로그램을 실행하거나 셀에 적용하지 않는다.

현재 내보내는 ActionBinding은 Host+intent다. 이것만으로 전체 Cell StepBinding의 완료/조건/절차 규칙을 새 셀에 이식하지 않는다. 원래 step/catalog digest를 통해 해당 정의와 함께 package를 구성·검증하는 경로가 추가로 필요하다.

## 화면

공정 원문을 저장한 뒤 작업별 등록 step을 선택한다. 선택 중에는 원문 편집과 섞이지 않게 하며, 선택 buffer는 메뉴 이동 뒤에도 유지한다. 이전 source에만 남은 선택은 명시적으로 제외하고, 바뀐 기준은 ‘현재 기준으로 검토’로 확인한다. 조회 응답의 대상과 요청 generation을 대조해 이전 draft의 늦은 응답이 현재 선택을 덮어쓰지 않게 한다.

바인딩 저장도 기존 pending key/body 회수 경로를 사용한다. 컴파일 입력 내보내기는 서버의 현재성 확인을 다시 거친다. 모든 연결 선택됨과 구조 확인됨을 운전 허가로 표시하지 않는다. 실제 장비용 표시명은 아직 제공되지 않아 등록된 step/Host/target ID를 보여준다.

## 검증과 다음 연결

시험은 commit 전/후 장애와 요청 회수, 잘못된 source/catalog/step/binding, 부분 선택, 원문 변경·제목 변경, 등록 구성 변경 뒤 재시작, 현재 역할 철회, matched export와 bundle 변조를 확인한다. 등록 구성 변경 시험은 명시적 저장 fixture이며 release activation 구현으로 세지 않는다.

브라우저는 실제 API로 step 선택·메뉴 이동 보존·저장 응답 유실/새로고침 회수·원문 변경 후 내보내기 거부·명시적 재검토/새 binding version을 확인한다. 등록 셀은 그대로 유지한다. 내보낸 bundle의 최종 이미지 컴파일도 별도 근거로 남긴다.

새 장비 capability/profile/교정에 대한 binding 생성기, 전체 StepBinding을 포함하는 package 조립·서명/배포/활성화, compiler preview의 UI 연결과 현장 인수는 남아 있다. 이 작성 경로는 현재 등록된 셀 작업을 재사용하는 범위이며 실물 검증을 대신하지 않는다.
