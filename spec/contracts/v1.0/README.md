# RX 계약·프로토콜 설계 기준판 v1.0

상태: v1.0 문서 revision 2026-09-14-neutral.1 · 문서 기준판. 제품 구현·상호운용 시험·실물 검증 완료를 뜻하지 않는다.

RX의 계약은 **누가 요청을 받아 기록했고, 장비에 어떤 동작을 전달했으며, 어떤 관측으로 결과를 판단했는가**를 표현한다. ROS·BT·통신 방식이 바뀌어도 이 의미를 유지하는 것이 목적이다.

## 읽는 순서

| 문서 | 답하는 질문 |
|---|---|
| [01 책임과 의미](01_responsibility_and_semantics.md) | 누가 소유하고 판단하는가? 작업 종류·접수·완료·불명은 무엇인가? |
| [02 식별·기록·복구](02_identity_durability_recovery.md) | 중복·재시작·제어권·저장 실패를 어떻게 다루는가? |
| [03 데이터·메시지·전송](03_data_and_protocol.md) | 어떤 데이터와 RPC를 어떤 순서·시간·버퍼 규칙으로 교환하는가? |
| [04 장비 binding·지원 조건](04_binding_and_admission.md) | 장비 모델·모드·시설에 공통 계약을 어떻게 연결하는가? |
| [05 결정 기록](05_decisions.md) | 대안 중 무엇을 왜 선택했는가? |
| [06 반례·검증·추적](06_scenarios_and_validation.md) | 유실·경합·다른 모델에서 성립하는가? 무엇을 후속 검증하는가? |
| [방법 조사 보고서](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/contract_research_2026-09-10/research_report.md) | 산업 표준과 최근 방법에 어떤 근거가 있는가? |

[독자 검토 기록](review_record.md) · [정규 표현 시험 벡터](canonical_vectors.md) · [프로토콜 manifest](protocol_manifest.json)

## 적용 범위

- 같은 현장 주 컴퓨터의 `rx-platform`과 `rx-solutions`, 단일 권위 Runtime, 하나 이상의 장비 Host를 대상으로 한다. 외부 미니컴퓨터는 C08의 부속 장비다.
- Rust 코어는 ROS를 의존하지 않는다. ROS·BT·native SDK·UI와 장비별 선택 패키지는 solutions가 소유한다. 두 이미지 안에 여러 프로세스가 있어도 된다.
- 유한 작업, 목표 상태 확보, 모드 전환, 제어 세션, 물리 효과가 있는 기동·종료를 포함한다. 고속 제어 sample은 solutions 내부에 남긴다.
- 실제 모델/펌웨어/교정/PLC 신호가 없는 경우 **공통 의미는 확정하고 해당 binding의 실행 허용은 보류**한다. 필요한 지원 profile과 미결을 기록한다.
- 다중 active Runtime의 자동 장애조치, 일반 인터넷 원격 조작, 안전 PLC 대체, hard real-time 보증, 물리 동작 exactly-once 보증은 v1.0 범위에 포함하지 않는다.

## 핵심 선택

1. 요청 접수·Host 준비·장비 접수·작업 결과를 구분한다.
2. 불명은 지식의 부족이다. 장비 정지나 실패를 뜻하지 않는다.
3. 영속 기록 전에 일반 물리 명령을 전달하지 않는다. 전달 가능성이 생긴 작업은 응답 유실 때문에 자동 재실행하지 않는다.
4. 결과의 확정과 자원/소재를 다음 작업에 넘기는 조건을 구분한다.
5. 내부는 gRPC+proto3 optional, 의미 동일성은 JCS+SHA-256, UI는 HTTP/JSON/SSE다.
6. 신호·모드·교정·관측·중단 능력은 버전 고정 binding profile로 선언하고 admission 때 검사한다.

`MUST/금지/필수`는 본 기준판 구현의 요구다. `예시`는 실물 설정값이 아니다. 명시된 software limit 기본값은 v1.0의 선택이며 장비 정지 성능 수치가 아니다. 장비별 시간·힘·속도·허용 오차는 검증된 profile이 제공해야 한다.

이 기준판은 06·11·15 및 contracts의 초기 계획/예시보다 계약 의미와 프로토콜 선택에 우선한다. 제품 범위(01–04), 장비 지원 정책(13), 두 이미지 경계(14)는 계속 유효하다. 변경은 새 revision과 영향 사례·migration 규칙을 남기며 기록된 작업의 의미를 소급 변경하지 않는다.

제조사 중립화의 범위·호환·hash 변경은 [문서 개정 기록](revision_2026-09-14.md)에 있다. 2026-09-10 검토 결과를 이 개정판의 새 실행 검증으로 주장하지 않는다.
