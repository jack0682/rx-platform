# 계약 설계 결정 기록

범위: RX 계약 v1.0 · 상태: v1.0 설계 결정 확정 · [방법 조사](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/contract_research_2026-09-10/research_report.md)

## 1. 결정 기준

기존 D01–D10을 동일한 장애 사례에 대입했다. 내구성·실물 정확성에 필요한 한계를 감추지 않는 대안을 먼저 선택하고, 두 컨테이너·Rust/ROS 경계·운영 부담을 비교했다. 아래 선택은 물리 실험으로 우월성을 측정한 결과가 아니라 명시된 조건에서의 설계 판단이다.

| 결정 | 비교 대안 | 선택·이유·비용 | 반례·연결 |
|---|---|---|---|
| D01 ID·중복 | caller operation UUID만 사용 / platform ID+caller key / 내용 hash만 사용 | **platform UUIDv7 + scoped key + 정규 의도 digest**. client 재시작과 서로 다른 반복 생산을 구별. 작은 tombstone 장기 보존 비용 수용 | SC02·05. 같은 key 다른 내용 거부, 다른 key 같은 내용 허용 |
| D02 작업 분류 | 모든 작업 execute/stop / 모델마다 전부 독자 API / 공통 envelope+종류별 body | **다섯 kind와 여섯 typed body**, 읽기는 별도. 공통 신뢰성 의미 유지하면서 mode/stream/기동 완료 의미 보존 | SC01·07·08·09·14. profile 작성 비용 필요 |
| D03 접수 | 메모리 접수 / platform만 durable / 양쪽 durable 단계 | **platform ADMITTED, Host PREPARED/SEND_ENTERED, native 접수 분리**. crash 창을 구별하고 불명 재호출 방지. Host 저장 volume·회수 protocol 추가 비용 | SC02·03. 물리 exactly-once는 제공하지 않음 |
| D04 제어권 | in-memory mutex / lease만 / Host fence+lease+인계 | **단일 Runtime, 자원 fence, Host grant, 인계 확인**. 옛 owner의 지연 명령을 수신점에서 차단. native 외부 client까지의 배타성은 별도 profile | SC03·08·14. 자동 active-active는 범위 밖 |
| D05 경합 | 먼저 온 성공/취소 채택 / timeout=실패 / 증거 기반 결과+별도 자원 상태 | **증거 기반 판정, immutable 결과와 DISPUTED incident**, cancel intent 독립. UI와 복구 절차가 한 단계 복잡해짐 | SC04. 성공 기록 후 충돌 증거도 격리 가능 |
| D06 사건 | best-effort topic / 모든 sample 영속 log / 제어 사건 journal+telemetry 분리 | **durable seq·재생·snapshot·갭·느린 소비자 종료**, 고속 telemetry 합치기 허용. 저장량과 제어 경로 block을 제한 | SC06·11. 무한 retention·무한 buffer 없음 |
| D07 동일성 | 원시 protobuf hash / deterministic protobuf / CBOR canonical / 의미 JCS | **명시적 정규 의도+RFC8785 JCS+SHA256**. UI/지원 도구 가독성, protobuf 구현 차이 회피. 양 언어 canonical validator와 fixture 검증 필요 | SC12. SI 단위·presence·default·집합/배열 의미 고정 |
| D08 원자성 | 다중 append / 분산2PC / 로컬 atomic state+event+outbox와 Host inbox | **두 로컬 transaction, 외부 물리 경계는 unknown으로 표현**. 첫 셀 SQLite adapter 선택, domain core는 저장 구현 비의존 | SC02·03·13. 저장 오류 때 신규 효과 차단, 현지 보호는 예외 |
| D09 시간 | 전역 timeout / peer wall clock / 용도별 timeout+receiver monotonic | **응답·준비·실행·freshness·grant·deadman 분리**. 만료 값은 작업별 검증. 재부팅 후 시계 연속성 추정 금지 | SC03·06·08·09. 일부 binding은 admission 준비 전 timing 시험 필요 |
| D10 호환·권한 | 미지 필드 무시 / minor만 비교 / 명시 schema hash·능력·역할 협상 | **exact negotiated schema + 엄격 입력 + mTLS identity/role**. 가짜 fallback motion 방지. 혼합 버전은 양 버전 지원 또는 coordinated upgrade 필요 | SC10·12. 기본 protobuf forward compatibility보다 보수적 |

## 2. 추가 전송·실행 선택

| ID | 결정 | 배제/후속 재검토 조건 |
|---|---|---|
| A01 | 내부 gRPC/HTTP2 + proto3 optional, UI HTTP/JSON/SSE | 자체 TCP·FFI는 측정 병목이 확인될 때. protobuf Editions는 Rust/C++ generator 호환 확인 후 |
| A02 | BT executor는 solutions, checkpoint와 operation mapping은 platform | BT를 신뢰성 코어로 합치지 않음. 다른 workflow engine은 같은 C02/C03 계약을 충족하면 대체 가능 |
| A03 | broker·Temporal/Restate/DBOS를 첫 셀 필수로 추가하지 않음 | 장기 주문/다수 셀/다수 소비자 요구와 운영 조직이 생기면 C02 위 또는 C04 외부에서 평가 |
| A04 | 산업 표준은 계층별 binding/외부 표현으로 사용 | OPC/PackML/VDA5050을 모든 로봇·CNC·stream에 일괄 적용하지 않음 |
| A05 | 고속 stream은 solutions 내부, 세션·판정만 platform에 기록 | Rust RPC를 1kHz actuator 제어 loop로 확대하려면 별도 실시간 설계 필요 |
| A06-R1 | 장비별 profile·의존성 선택, 동일한 admission 조건 적용 | 2026-09-14 개인 프로젝트의 제조사 중립화로 이전 포함 정책을 대체. 미검증 모델 자동 활성 금지는 유지 |

## 3. 결정 변경 조건

새 증거가 요구를 바꾸면 decision ID를 supersede한다. 기존 문서·operation은 원래 profile/schema/digest 의미로 읽어야 한다. schema 변경 시 translator가 의미를 보존한다는 계약 시험 또는 구버전 reader 보존이 필요하다. DB migration은 사전 backup·세대·rollback 절차와 함께 별도 승인된 구현 단계에서 다룬다.

예를 들어 PLC가 durable request ID와 결과 이력을 제공하고 반복 제출을 확실히 dedup한다면, 해당 profile의 불명 재제출 규칙을 차기 minor/별도 capability로 확장할 수 있다. 그 전까지 ‘아마 멱등’이라는 추정으로 D03을 완화하지 않는다.

## 4. 후속 검증을 남기는 이유

설계 기준판은 **어떤 근거가 있어야 실행을 허용하는지**를 확정한다. 아직 없는 PLC 신호·모델 교정·정지 성능을 임의로 결정하면 설계 완료가 아니라 사실 조작이 된다. 남은 binding 조건은 [검증 목록](06_scenarios_and_validation.md)의 담당 역할·산출물·영향 범위로 관리한다. 이 미확정은 공통 계약의 결론을 미루는 이유가 아니며 해당 binding의 납품 범위를 제한하는 조건이다.

현재 문서는 제조사 중립 문서 개정판이다. 표의 이전 설계 감사 결과는 [원래 문서](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/contracts/v1.0/05_decisions.md)의 당시 범위에만 해당하며 이번 개정의 실행 시험 결과가 아니다.
