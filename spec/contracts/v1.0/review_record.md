# v1.0 설계 검토 기록

2026-09-10 · 검토 종류: 독립 독자 검토와 문서 사건 추적 · 구현·실물·성능 시험 아님.

## 1. 검토 범위

의미/복구 독자는 01·02·06을 중심으로 중복 동작·잘못된 완료·자원 해제 반례를 검토했다. 데이터/protocol 독자는 03·04·05와 연결 규칙을 중심으로 독립 Rust/C++ 구현의 해석 차이를 검토했다. 각 독자는 문서를 읽었으며 수정·제품 실행을 수행하지 않았다.

## 2. 지적과 반영

| 지적 | 반영한 규칙 |
|---|---|
| 권한 검사 후 늦은 native 제출 | 검사·SEND_ENTERED·native 진입과 revoke를 같은 command gate로 직렬화, 진입 호출이 남으면 인계 금지 |
| PREPARED 취소와 Authorize 경합 | Host VOIDED_BEFORE_SEND terminal CAS, P outbox NEW/VOIDED/EMIT_ENTERED CAS |
| ticket 교체 후 sample 역전 | session/source boot 전체 수용 순번과 자원별 단일 native 적용 consumer/gate |
| 종료된 activation의 새 slot 제출 | T1의 run/checkpoint/activation/executor 적격성 CAS, 기존 mapping 조회와 신규 admission 구별 |
| 단독 operation과 run 검사 충돌 | setup/manual 단독 경로와 executor/run 경로 구분, 생산 우회 금지 |
| production·cancel·stream native 횟수 혼합 | production operation, cancel_id, session/source_seq, protection incident의 단위 분리 |
| schema hash 미정의 | 단일 protocol manifest artifact와 규범 문서 digest 고정 |
| enum/oneof/optional canonical JSON 차이 | 정확한 JSON 투영·Evidence/Receipt 별도 hash prefix·문서 fixture |
| Host→P freshness 비교 불가 | 같은 host boot CLOCK_BOOTTIME 검증과 취득 불확실성을 포함한 age 상한 |
| delivery/evidence journal 순번 충돌 | 별도 journal ID·seq 공간, 각각 dedup |
| mutation CAS·dedup 순서 차이 | 현재 접근권→key 동일성→신규 요청 CAS, RPC별 CAS 대상과 revision 위치 명시 |
| 인증 session과 control session 혼합 | control session은 해당 operation ID, grant/source/schema에 결합 |
| 묶음 자원의 scalar fence | max(기존 자원별 fence)+1을 묶음에 원자 적용, equal fence는 동일 요청 재생만 |
| snapshot pagination·exclusive after 혼동 | 별도 GetSnapshotPage, after=through, 첫 사건=through+1 |
| 빈 snapshot의 seq=0 불가 | 빈 through=0과 after=0 허용 |
| Observation 필수/presence 불일치 | 01과 03 통일, 원본 boot/seq 부재의 제한 명시 |
| cancel 결과 상관관계 누락 | 별도 cancel invocation ID, Correlation.cancel_id, 미호출 tombstone stage |
| ProfileFinding·Host 결과 권위 불일치 | 구조적인 finding code/support ID, Host는 receipt/evidence만 반환하고 P가 결론 확정 |

두 독자는 수정 부분을 재검토했고, 제기한 지적이 문서상 해소됐다고 보고했다. 마지막 확인 범위에서 추가 P1/P2 모순을 보고하지 않았다. 이는 전체 가능한 장애에 대한 증명이나 납품 승인 대신 사용할 수 없다.

원장 근거 조회·telemetry 읽기 API, key의 activation binding, executor session 결합, checkpoint/reconcile 세부 규칙은 통합 검토에서 추가 확인했다. 제품 코드·컨테이너·패키지 설치·실물 명령은 만들거나 실행하지 않았다.

## 3. 최종 상태

- C01–C08 책임, D01–D10 선택, SC01–SC14 및 재시작/저장 변형에 대해 문서 기준판을 작성하고 검토했다.
- 계약의 공통 의미·복구·전송·표현 규칙은 v1.0으로 확정한다.
- V01–V11의 구현·형식 모델·실물 검증은 미실행 상태로 유지한다.
- 실제 모델/펌웨어·교정·PLC 신호·timing이 없는 profile은 생산 admission을 허용하지 않는다.
- 형식/링크/manifest/fixture의 파일 검사는 별도 문서 무결성 기록으로 남긴다.
