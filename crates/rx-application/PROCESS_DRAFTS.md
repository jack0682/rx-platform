# 공정 초안·버전·구조 검사

2026-09-11. 편집 중인 공정은 설치된 CellConfiguration/Run과 별도로 보존한다. 저장은 운전 구성 변경, package activation, qualification 또는 native 실행이 아니다.

## 책임

P application은 현재 사용자·셀 접근권, 초안 revision/CAS, 요청 동일성, 내용과 이력을 저장한다. S의 공정 작성 UI와 compiler는 공통 source model을 사용한다. 구조 검사 코드는 `rx-process-contract::source_validation`에 두어 P의 작성 경로와 S의 실제 compiler가 같은 규칙을 사용한다. P가 ROS/BT/compiler 실행 엔진에 의존하지 않는다.

구조 검사는 source schema, flow/node의 유일성·연결·도달 가능성, 순환/재귀, 유한 반복·대기, 조건 구조와 4096개 전개 한계를 확인한다. 필요한 action binding 이름과 procedure reference를 돌려주지만 실제 장비·intent·자원 충돌·권한을 검증했다고 하지 않는다. S compiler는 이후 실제 bindings의 존재/정규화, 병렬 자원 충돌과 기존 확장을 계속 검사한다.

검사 결과에는 validator 이름과 해당 source/model 코드의 digest를 기록한다. 저장된 결과가 어느 검사 구현에서 나왔는지 식별할 수 있다. 이것은 외부 검증 패키지나 제품 qualification 서명이 아니다.

## 미완성 문서와 저장 원자성

초안 document는 최대 512 KiB의 canonical JSON 값이다. 따라서 구문상 JSON이지만 source 의미가 미완성인 내용도 저장하고, 구조 오류를 그 버전의 결과로 남길 수 있다. Source 타입으로 해석할 수 없는 document도 실행 가능한 source로 바꾸지 않고 형식 오류로 기록한다.

`PreparedSave::prepare`가 문서 크기·제목·digest·구조 검사 결과를 만든다. API는 이 작업을 별도 CPU worker에서 수행한다. 준비 객체의 내부 값은 외부 wire에서 만들 수 없으며, writer는 commit 직전 현재 Engineer 역할·단말/셀 범위를 다시 확인한다.

한 transaction에서 내용별 immutable document, 최신 version, version history, 작은 목록 index, 감사 사건과 같은 요청의 결과를 저장한다. document는 cell+content digest로 보존하여 같은 내용의 제목 변경마다 본문을 복제하지 않는다. 과거 version은 같은 문서를 계속 참조한다.

같은 key와 같은 body를 다시 보내면 원래 version을 반환한다. 같은 key에 다른 내용은 충돌이다. 기존 revision과 다르면 새 요청이라도 현재 내용을 덮어쓰지 않는다. 권한 검사는 이전 응답 회수보다 먼저 수행한다. API 화면에서의 재시도도 저장했던 key/body를 유지한다.

## API

| 요청 | 용도 |
|---|---|
| POST `/api/v1/process-drafts` | `request_key`와 `command{id,cell,expected,title,document}` 저장. 신규는 expected=null, 변경은 현재 revision |
| GET `/api/v1/process-drafts?cell=...&after=...` | Engineer/Verifier의 허용 셀 목록. 50개와 다음 ID. 원문은 목록에 싣지 않음 |
| GET `/api/v1/process-draft?cell=...&id=...` | 최신 원문과 그 버전의 구조 검사 |
| GET `/api/v1/process-draft?cell=...&id=...&revision=...` | 해당 과거 버전의 원문과 결과 |

다른 셀로 기존 초안을 이동시키는 update는 허용하지 않는다. 재사용은 새 draft ID로 복사하고 대상 셀에서 별도로 관리한다. Observer/Operator 역할만으로 미배포 초안을 조회·수정하지 못하며 Verifier는 조회, Engineer는 저장할 수 있다. 실제 Cell/Run revision, grant, budget, operation outbox는 바꾸지 않는다.

## 편집 화면

‘공정 설계’는 Engineer/Verifier에게 표시한다. source flow와 순차·병렬·분기·반복·호출·작업·대기·개입 노드를 선택하고 연결한다. 시작 노드, 자식 순서, 작업 binding 이름, 조건 이름과 기한을 편집할 수 있다. 조건식 및 procedure artifact의 상세 편집은 현재 고급 JSON 편집 경로다.

화면의 구조 preview는 최대 256개 항목과 제한된 깊이로 표시하며, 미완성 그래프의 순환·중복 경로가 브라우저를 무한히 확장시키지 않게 한다. 전체 의미 판정은 저장 버전의 공통 validator 결과를 따른다. 구조 오류는 위치와 한국어 설명을 보여주고 원래 code/message를 남긴다.

편집 buffer는 현재 계정/installation/store generation과 셀에 묶어 메모리에 보존한다. 메뉴를 바꿔도 사라지지 않으며 source/condition JSON의 적용 전 입력도 유지한다. 적용 전에는 다른 편집·저장을 잠그고, 적용 또는 취소를 명시적으로 선택한다. 미저장 내용이 있으면 페이지를 떠날 때 경고한다. 이 버퍼는 브라우저 재시작 복원용 영속 문서 저장소가 아니므로 명시적 저장이 필요하다.

저장 응답을 잃으면 기존 공통 pending request 경로로 같은 요청을 회수한다. 충돌은 로컬 편집을 보존하며 현재 서버 버전 또는 지정한 과거 version과 비교할 수 있다. 비교가 편집 내용을 바꾸지 않는다. ‘변경 버리기’와 새로 열기를 통해 최신 서버 내용을 선택한다. 복사는 별도 draft ID/revision 1로 저장한다.

## 패키지·실행으로 넘길 경계

화면은 현재 source JSON을 내보낸다. 내보낸 source는 S의 기존 `rx-process-compile`/`compile_package` 경로와 실제 binding을 사용해 다음 단계로 넘길 수 있다. 이 단계에서 UI가 실제 binding을 자동 선택하거나 패키지를 서명·배포·활성화하지 않는다. 구조 통과와 필요한 작업 이름은 그런 승인을 뜻하지 않는다.

S compiler의 확장·node/source identity·조건/자원 검사와 기존 BT 실행/복원 시험을 계속 사용한다. frozen base/cell 규범과 네 optional wire binding은 유지한다. 공유 SDK는 새로운 source-validation 모듈을 포함한 80개 payload로 수출한다.

## 검증과 남은 범위

시험은 저장 전/후 장애, 동일 요청 회수, 불완전 source의 저장, 이전 version 보존, 경쟁 update, 현재 Engineer 권한 철회와 다른 셀 비노출을 확인한다. 브라우저에서는 실제 P API로 생성·수정·충돌·이력 비교·복사·응답 유실/새로고침 회수와 편집 buffer 보존을 검증한다. 활성 셀 구성은 동일하게 유지돼야 한다.

장비 capability/binding의 시각적 선택, 완전한 조건·개입/복구 편집기, compiler preview와 package 생성/서명/배포의 UI 연결, 변경 영향 분석, 검토 승인 흐름, 큰 목록의 DB page index와 보존 정책은 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.

실제 브라우저에서 저장·내보낸 source와 모의 binding을 최종 S 이미지의 compiler로 처리한 결과도 남겼다. 결과는 COMPILED_NOT_QUALIFIED이며 package 서명·활성화·native 실행을 수행하지 않았다.

현재 셀의 등록 step을 선택하는 [장비 작업 바인딩](DRAFT_BINDINGS.md)과 matched compile input export를 연결했다. 새 장비/profile의 binding 생성·package 승인은 여전히 별도다.
