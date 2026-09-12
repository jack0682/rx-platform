# 개입 사건의 접수·보류·알림 확인

상태: 사건 생성/조회와 알림 확인을 P transaction, executor mTLS와 개발용 HTTP·운영 화면에 연결했다. 후속 단계에서 [정책 기반 절차 보고·개인별 종료/인수와 REVALIDATING](PROCEDURE_REPORTS.md)을 연결했다. 복구 plan·clearance·restart는 아직 미완료다. 확인 버튼으로 이 상태를 건너뛸 수 없다.

## 사건을 열 때

현재 Operator/RecoveryLead/Executor/Host 권한과 요청 셀 접근권을 확인한 뒤 전체 key/body를 비교한다. 책임자는 실제 활성 RecoveryLead 계정이어야 하며 영향 셀에 접근할 수 있어야 한다. 제공한 operation ID는 실제 영향 셀의 기록이어야 한다. 소재 ID는 보고된 참조이며 이 API가 소재 식별·위치·지지를 확인한 MaterialState를 만들지 않는다.

현재 authority 단위가 cell이므로 영향은 현재 cell의 영역·공유 자원으로 연결된 cell closure까지 적용한다. 알 수 없는 scope/소재 참조는 scope_uncertain으로 보존한다. 요청한 좁은 scope를 그대로 안전 경계로 취급하지 않는다.

| 사건 유형 | 원자적 처리 | 최초 상태 |
|---|---|---|
| DIAGNOSTIC_ONLY | 영향 cell에 non-latched block과 case membership 추가. epoch·기존 ACTIVE mandate 유지 | OPEN |
| PLANNED_ACCESS / FAULT_RECOVERY / MAINTENANCE / CHANGE_REVIEW | 기존 invalidation 경로의 epoch/fence/mandate 철회·미진입 permit 봉인 및 case membership | CONTAINMENT_PENDING |

진단 사건도 관련 새 admission을 보류한다. 기존 invocation의 상관 결과는 계속 기록할 수 있다. 실제 물리 접근·변경·연속성 상실이 보고되면 진단만으로 끝낼 수 없으며, 해당 보고·전환 경로는 후속 구현이다. 현재 진단 사건을 자동으로 해제하지 않는다.

새 case와 block/허가 처리·사건·응답은 같은 commit이다. 응답 유실의 동일 key/body는 같은 case와 처음 응답을 회수한다. 다른 case의 block/membership을 제거하지 않는다. 공유 영향 셀은 같은 case를 조회할 수 있다.

## 알림 확인

Case.Acknowledge는 현재 계정의 알림 확인만 기록한다. actual actor, 기록 시각, 보고된 RFC3339 UTC 시각, 당시 case revision과 scope, 내용 주소 rx.procedure-assertions.v1 artifact를 저장한다. assertion에는 authenticated-notification-ack와 빈 physical_claims를 명시한다. UTC는 기록용이며 명령 유효성이나 접근 조건을 계산하지 않는다.

case record ID 목록과 revision은 증가하지만 case 상태, 참여자, cell epoch, block, mandate, operation outcome, 접근/reset/재시작 허가는 바꾸지 않는다. 같은 요청의 재생은 확인 기록을 늘리지 않는다. 같은 key의 다른 body, stale case revision, 권한 없는 계정은 거부한다. 이 ACK 경로를 범용 ProcedureRecord 처리로 표시하지 않는다.

## 통신과 화면

- frozen Cell.OpenCase/GetCase는 등록된 executor identity에 연결했다. OpenCase는 제한을 추가하므로 유효한 과거 expected_cell_revision을 CAS로 거부하지 않지만 key의 전체 의도에는 보존한다.
- 개발 HTTP는 `/api/v1/cases`, `/api/v1/case`, `/api/v1/cases/open`, `/api/v1/cases/acknowledge`다. 기존 개발용 cookie·same-origin·body 규칙을 사용하며 frozen 운영 HTTP 전체 투영의 완료를 주장하지 않는다.
- 운영 화면은 영향 case 목록과 상태·책임자·제한·확인 기록을 보여 준다. ACK는 기존 브라우저 pending-key 흐름을 사용한다. 절차 catalog/사건 생성 UI, 물리 절차 입력과 재시작 UI는 후속이다.
- RecordProcedure는 후속 단계에서 typed assertion/현재 actor·policy 검사와 연결했다. recovery/clearance RPC는 미완료이며, 해당 동작을 ACK로 대체하지 않는다.

## 검증

core 시험은 생성 transaction rollback/응답 유실·동일 case 회수, latched와 diagnostic 보류의 차이, UNKNOWN 결과 보존, ACK의 비승격, 다른 case의 제한 유지와 중복 key를 확인한다. HTTP는 readonly 계정 거부와 같은 ACK 재생, mTLS는 executor Open/Get와 미지원 procedure 거부를 검사한다.

브라우저는 실제 개발 API에 case를 준비하고 ACK 응답을 commit 뒤 유실한다. 페이지 재로딩 후 같은 body/key로 회수하고 record1개·CONTAINMENT_PENDING·동일 epoch/block을 확인한다. desktop/mobile 표시와 JavaScript 오류도 확인한다. 절차 참조와 셀은 합성 자료이며 실제 장비나 물리 접근 시험은 없다.

후속 작업에는 typed external procedure evidence·stale-CAS에서도 사실 보존, 절차/참여자/인수 정책, recovery plan과 unique step slot, clearance cohort, 명시적 restart/비운전 종료, 원장 paging 및 production deployment가 포함된다.
