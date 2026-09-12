# 기록된 셀 상태와 운영 API 서비스 세션

2026-09-11. 셀의 모드·운전 자격 상태·차단 생성 버전을 저장하고, frozen `Cell.Inspect` 및 사람이 사용하는 HTTP 조회에 연결했다. 운영 API 서비스도 `OPERATOR_API`로 협상할 수 있다. 이 서비스의 인증은 사람의 시작·복구 의도를 대신하지 않는다.

규범 근거는 [CellContext와 역할](../../spec/cell_operations/v1.0/04_protocol_integration_ui.md), [base Session/PeerHello](../../spec/contracts/v1.0/03_data_and_protocol.md)다. 규범 8개 파일과 manifest는 그대로다.

## 셀에 저장하는 정보

| 정보 | 기록 방식 | 해석 |
|---|---|---|
| mode | SETUP / AUTOMATIC / RECOVERY / MAINTENANCE | RX가 처리 중인 운영 문맥. 로봇·PLC의 실제 모드 셀렉터 값은 별도 관측/조건이다 |
| commissioning | NOT_COMMISSIONED / COMMISSIONED / REVALIDATION_REQUIRED | 현재 저장된 사용 자격과 재검토 필요 상태. 운전 조건 PASS나 물리적 안전 판정과 다르다 |
| block.created_revision | 차단을 처음 저장한 해당 셀의 실제 revision | 조회 시점 revision으로 덮어쓰지 않음 |
| block.case_id | 해당 차단을 만든 사건 ID | 다른 사건 차단이나 과거 종료 기록과 혼합하지 않음 |

`Cell`과 `Block`의 내부 저장 모델에서 새 metadata는 optional이다. 이전 기록을 읽을 때 값이 없으면 None을 유지한다. 새 셀과 새 차단에는 실제 기록 경계에서 값을 채운다.

## 상태를 바꾸는 경계

| 사건 | mode | commissioning |
|---|---|---|
| 새 셀 등록 | SETUP | NOT_COMMISSIONED |
| 정확한 구성·근거 qualification 등록 | 유지 | COMMISSIONED |
| 모든 Host Arm 확인 뒤 최초 생산 시작 commit | AUTOMATIC | 현재 자격 재확인 |
| 모든 Host Arm 확인 뒤 setup 시작 commit | SETUP | 현재 자격 재확인 |
| latched invalidation | 단순 HOLD/재부팅은 기존 문맥 유지 | 기존 COMMISSIONED는 REVALIDATION_REQUIRED |
| 비진단 개입 사건 생성 | 정비 사건은 MAINTENANCE, 그 외 RECOVERY | 기존 자격의 재검토 필요 |
| 실제 절차 변화 보고 | 사건에 맞는 MAINTENANCE/RECOVERY | 기존 invalidation 규칙 적용 |
| 비운전 종료 commit | MAINTENANCE | OUT_OF_SERVICE 차단과 재검토 상태 유지 |

진단 사건이나 알림 ACK는 운영 모드·자격을 바꾸지 않는다. 시작을 요청했다는 이유로 먼저 AUTOMATIC을 표시하지 않는다. 모든 Arm 확인과 Run/Mandate commit 때 셀 상태도 같이 바뀐다. 따라서 시작 전 셀 revision은 이후 작업의 CAS에 재사용할 수 없다.

한 셀의 scalar 운영 문맥과 모순되게 서로 다른 목적의 run을 동시에 활성화하지 않는다. 이미 실행 중인 생산 run이 있을 때 setup 시작을 거부한다. 이것은 일반적인 병렬 공정·자원 스케줄링 전체의 구현을 뜻하지 않는다.

이 모드 필드는 native ModeGoal이나 물리 셀렉터를 조작하지 않는다. 장비 실제 모드·진입 조건·운전 유지 조건은 profile/조건 근거로 별도 확인해야 한다. 화면에는 ‘RX 운영 모드’라고 표시한다.

## 차단과 revision

차단은 해당 Cell을 처음 갱신하는 revision에 결합한다. 예를 들어 invalidation이 revision 8에 차단을 만들고 같은 transaction에서 사건 membership을 revision 9에 저장해도 `created_revision=8`이다. 이후 ACK·조회·다른 변경으로 이 값을 수정하지 않는다.

개입/절차 처리기는 자신이 만든 차단에만 case_id를 연결한다. 공통 HOLD/RuntimeRestart/OUT_OF_SERVICE 차단에는 임의의 사건 ID를 지정하지 않는다. non-operating close가 다른 사건의 차단을 지우지 않는 규칙은 유지한다.

## 공개 조회

| 경로 | 인증과 범위 | 결과 |
|---|---|---|
| gRPC Cell.Inspect | 등록된 mTLS 인증서, 현재 base session, 정확한 셀 manifest/definition 협상, 현재 셀 접근권 | frozen CellContext |
| GET `/api/cell/v1/cells/{cell_id}/inspect` | 기존 HTTP 사용자 세션, 현재 역할/셀 범위 검사 | 같은 CellContext의 RX JSON |
| 기존 `/api/v1/overview` 및 운영 화면 | 현재 사용자별 projection | 저장된 mode/commissioning 추가 표시 |

cell_id에 `/`가 들어가면 path segment를 percent encoding한다. 예: `cell/a` → `cell%2Fa`.

gRPC Inspect는 mutation key나 expected revision을 소비하지 않는다. 지정하면 거부한다. 셀과 접근권·협상 binding을 같은 writer read transaction에서 확인하고, 별도 트랜잭션에서 mode나 block 정보를 다시 가져와 섞지 않는다.

converter는 존재하지 않는 metadata, 0/future created_revision, 중복 block/open_case ID, scope vector 불일치, COMMISSIONED와 qualification 참조 불일치를 거부한다. metadata가 없는 이전 기록은 UPGRADE_REQUIRED다. 다른 저장 무결성 오류는 DATA_LOSS/STORE_FAULT로 분리한다. 숫자·enum·optional은 frozen RX JSON을 사용한다.

이 CellContext 조회는 필터된 제어 원장 cursor나 SSE를 제공하지 않는다. 전체 CellJournal wire projection/paging/구독은 별도 후속이다.

## 운영 API의 서비스 신원

`Role::OperatorApi`는 전송 서비스 역할이다. 일반 Operator/RecoveryLead와 같은 enum 값으로 취급하지 않는다. 등록 계정은 OperatorApi 및 선택적인 Observer 역할만 가진다. 다른 사람/Host/Executor 권한을 섞은 계정의 운영 API 협상을 거부한다. 이 서비스 계정은 일반 사용자 로그인 세션으로도 열 수 없다.

- Session.Open은 `OPERATOR_API`를 지원한다. evidence journal/last_seq는 허용하지 않는다.
- 인증 binding은 인증서 fingerprint·installation·store generation·release digest에 결합한다.
- 동일 boot/binding/current session의 재접속은 같은 세션이다. 새 boot/binding은 이전 서비스 세션을 폐기하고 셀 협상 상태를 비운다. 퇴역한 boot의 재진입을 거부한다.
- Cell.Open은 현재 접근 가능한 정확히 한 definition을 협상한다. 모호한 definition이나 다른 셀은 허용하지 않는다.
- Cell.Inspect는 매번 현재 계정·역할·범위와 session/certificate/definition을 재검사한다.
- 이 서비스의 연결·재접속은 셀 epoch/Run/Mandate나 실행기 session을 바꾸지 않는다.
- 서비스 인증만으로 Cell.OpenCase, 작업/복구/시작, Executor GetRun/Submit 권한을 얻지 않는다.

현재 인간의 실제 동작 입력은 직접 terminal HTTPS와 개발 HTTP가 인증된 사용자 Identity로 application을 호출하는 경로다. 운영 API 서비스의 gRPC 쓰기에 사람이 누구인지 연결하는 위임/세션 binding은 아직 구현하지 않았다. 서비스를 RecoveryLead나 Executor로 승격해 이 공백을 메우지 않는다.

후속 연결은 현재 사용자 세션·서비스 세션·등록 단말의 근거를 구별해야 한다. body의 actor/session 문자열을 신원 증거로 받지 않고, 매 mutation마다 현재 사람 권한을 검사한 뒤 기존 idempotency cache를 조회해야 한다. 이 경계가 연결된 뒤 frozen PrepareClose/CloseWithoutRestart 등 인간 쓰기 RPC를 활성화한다.

## 저장소와 rollback 경계

이 단계에서 schema4 barrier를 도입했다. 현재 사용자·단말 binding 구현은 [schema5](TERMINAL_IDENTITY.md)로 확장되었다. 새 필드를 모르는 기존 runtime은 `version > 3` 검사에서 이 저장소를 거부한다. 단순 바이너리 교체로 오래된 decoder가 새 Cell 문서를 읽거나 일부 갱신하도록 두지 않는다.

0004 migration은 기존 entity/request/event/outbox/control 자료를 바꾸지 않고 compatibility barrier만 올린다. 값이 없던 mode/commissioning/block 생성 시점을 추정하지 않는다. 이전 기록의 검토·재연결/명시적 metadata 보완은 후속 절차다. 이전 바이너리 복귀와 이전 데이터 snapshot 복원은 같은 작업이 아니며, 전체 P/H 복원·native 결과 재조정 명세를 생략할 수 없다.

시험은 실제 schema 3 테이블과 문서 bytes를 만들고 schema 4 개방·backup 뒤에도 bytes가 같은지 확인한다. 현재 decoder보다 높은 버전의 개방도 거부한다. 과거 코드 archive의 `version > 3` 조건은 별도로 확인했다. 실제 배포 업데이트/현장 복원 시험을 완료했다는 의미는 아니다.

## 검증과 남은 범위

application 시험은 시작/개입 mode·자격 변경, 실제 차단 생성 revision, 서로 다른 실행 목적 충돌, 운영 API 재접속/혼합 역할·실행기 권한 불변을 확인한다. HTTP 시험은 현재 사용자 범위, frozen JSON, missing legacy metadata/future revision 거부를 확인한다. 실제 TLS fixture는 OPERATOR_API 협상 전후 조회·scope 거부·쓰기/실행기 권한 거부·새 boot/퇴역 boot를 확인한다.

운영 화면에는 자격 기록 유무와 재검증 필요를 구별하고 RX 운영 모드를 표시한다. 브라우저는 실제 로그인·미등록 상태·모드 표시·응답 유실 복원·모바일 범위를 확인했다. qualified physical cell의 시각적 인수는 미수행이다.

남은 핵심은 인간 세션/단말을 서비스 호출에 연결하는 경계, frozen 인간 쓰기 RPC, 명시적 모드 변경/qualification 갱신·legacy metadata 보완, recovery/restart와 전체 journal/제품 배포다. 실제 로봇/PLC 모드나 안전기능의 검증을 이 구현으로 주장하지 않는다.
