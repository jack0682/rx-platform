# 절차 사실 기록과 상태 승격

외부 절차 보고를 저장하는 동작과 그 보고로 case 상태를 승격하는 동작을 분리했다. RecordProcedure는 native reset·격리·로봇 이동 명령을 실행하지 않는다. 현재 구현은 PROCEDURE_ACTIVE와 REVALIDATING까지이며 READY_FOR_RESTART/생산 재시작은 별도 후속이다. 비운전 clearance·종료는 [별도 경로](NON_OPERATING_CLOSURE.md)로 연결했다.

## 정책을 신뢰하는 경계

Procedure Policy는 cell/definition/envelope, 외부 절차 문서와 의존 근거 artifact, 허용 case 유형, entry 조건, 단계별 action/actor/조건/필수 관측 source와 보고 최대 age를 포함한다. 실제 bytes/hash/schema를 확인해 내용 주소로 보존한다.

Engineer 권한만으로 정책을 승인하지 않는다. QualificationAuthority의 별도 verify_procedure가 이를 검증해야 하며 기본 구현은 항상 거부한다. 매 상태 승격 때도 현재 authority와 현재 구성 binding을 재검사한다. 이 phase의 정책 admission 시험은 명시적 simulation authority를 사용한다. 실물 release/package verifier와 실제 절차 문서 검증을 끝낸 상태가 아니다.

## 보고 형식과 원장

rx.procedure-assertions.v1에는 case/기존 revision/절차 digest/actor/action/UTC/scope/source, source-event ID, step, 사람·물리 대상과 typed claim, 근거 ID·관측 시각/유효 기간·reported/unknown을 담는다. 앞선 notification ACK 형식은 optional field 기본값으로 읽을 수 있다. UTC는 기록용이며 유효성 판단에는 현재 P clock을 사용한다.

현재 인간 절차 보고 source는 인증된 Operator/RecoveryLead 계정이다. actor/source가 실제 identity와 일치하고 record metadata와 artifact bytes/hash/size/schema가 맞아야 한다. WorkStarted/WorkFinished는 해당 actor 자신의 작업 보고이며 타인의 종료로 대신할 수 없다. 장비 조건은 기존 Host FactRecord의 source/session/age/quality와 정의된 조건으로 검사한다. free-text claim을 machine condition으로 실행하지 않는다.

record ID·source event·내용은 보존한다. 같은 ID의 다른 내용은 기존 기록을 덮어쓰지 않고 충돌/차단을 남긴다. 모든 report는 기존 control/audit 원장에 기록된다. 새로운 physical fact는 필요한 closure invalidation을 실행한다. 진단 case에서 실제 작업이 보고되면 FAULT_RECOVERY와 latched 상태로 바뀐다. 구성 보고는 production qualification을 무효화한다.

## stale 요청의 처리

현재 인증/접근권과 source/ID/내용을 검증한 뒤, 실제 보고를 먼저 기록한다. 요청한 case/cell revision이 오래되면 새 허용 상태는 만들지 않고 transition_error=STALE_REVISION을 반환한다. 이미 저장한 사실과 차단은 rollback하지 않는다. 같은 key/body는 최초 receipt를 반환하므로 physical change나 case record를 다시 추가하지 않는다.

저장 자체의 실패는 사실이 기록됐다고 응답하지 않는다. commit 후 응답 유실은 원래 key로 회수한다. 허가 확대의 실패와 사실 수용은 Receipt의 facts_recorded/record/transition_error로 구분한다.

## 상태 판단

- ACK는 notification 기록에 그친다.
- EntryConditionsReported는 현재 책임자의 RecoveryLead 역할, admitted policy와 actor/step, 전체 영향 scope, 현재 Host fence ack, P의 entry 조건과 근거·age가 모두 필요하다. reported=true만으로 진입하지 않는다.
- WorkStarted/WorkFinished는 외부 사실을 보존하고 개인별 진행을 기록한다. 절차 상태 확대에는 현재 policy와 이전 진입 context의 연속성이 필요하다. 다른 원인으로 epoch가 바뀌면 옛 entry 기록을 사용할 수 없다.
- PersonnelAccounted/HandoverAccepted는 현재 lead가 전체 비어 있지 않은 참가자 집합을 확인해야 한다. 한 사람이 끝났거나 빈 명단이라는 이유로 다른 사람/미확인 상태를 해제하지 않는다.
- 모든 알려진 참여자의 작업 종료·인원 확인·인수가 모이면 REVALIDATING이다. 이것은 restart 준비 완료가 아니다.
- Isolation/Reset/Configuration 보고는 기존 entry를 무효화하며 새 확인이 필요하다. configuration_changed는 새 진입 승격도 차단하고 후속 변경/qualification 절차를 요구한다.

## API

개발 HTTP `/api/v1/cases/procedure`는 typed Submission과 inline assertions를 받고 같은 P transaction을 실행한다. 승격 실패에는 HTTP 409/403/422 등과 facts_recorded, case_id, record_id, 전체 receipt가 포함된다. 사실을 저장한 뒤 오류가 반환됐다는 점을 호출자가 보존해야 한다.

frozen Cell.RecordProcedure는 이미 저장된 case/actor-bound assertion artifact를 받는 구조다. 현재는 service 계정에 Operator/RecoveryLead 역할을 추가해 인간 보고를 보내는 동작을 거부한다. 인간의 gRPC 보고에는 검증된 사용자·등록 단말을 OPERATOR_API 호출과 연결하는 후속 binding이 필요하다. 실제 사용자의 직접 HTTPS/개발 HTTP가 같은 application 처리기를 사용한다. 상태 승격 오류에는 rx-procedure-facts-recorded, rx-procedure-record-id, rx-procedure-case-id metadata를 붙인다. 첫 인간 보고의 inline bytes 경로는 현재 개발 HTTP다.

CaseDetail은 notification ACK와 procedure record, 개인별 procedure progress를 구별한다. 운영 목록의 ACK 건수는 physical report 건수와 섞지 않는다. 실제 절차 입력·검토 화면은 후속이다.

## 검증 범위와 한계

core는 stale-CAS/rollback/commit 응답 유실의 사실 보존, 실제 fence/condition 근거 전의 진입 거부, 두 작업자의 독립 종료·전체 인원/인수, 외부 epoch 변경 후 옛 entry 거부를 검사한다. HTTP는 stale report가 오류 응답이어도 record ID와 한 개의 사실·차단을 유지하는지 확인한다. 기존 ACK/UI·실행/인계/서비스 경로를 회귀 검사한다.

실제 작업과 장비 상태를 RX가 관측만으로 안전 인증한 것이 아니다. 정의된 외부 절차·보호 기능의 source와 근거를 기록·검사하는 소프트웨어 경계다. 실제 policy admission·물리 절차·제품 인수는 미수행이다. recovery plan/guarded operation, restart와 무진입/불명 범위 종료, 새 구성 qualification, 전체 operator service identity/화면·artifact catalog/원장 보존은 계속 남아 있다.

물리 변화 보고는 기존 인원 확인·인수 기록을 무효화한다. 이미 저장된 record가 다른 요청 key로 재전송되어도 이전 작업 전이를 다시 적용하지 않는다. 종료 후 새 사실의 보존은 [비운전 종료 명세](NON_OPERATING_CLOSURE.md)를 따른다.
