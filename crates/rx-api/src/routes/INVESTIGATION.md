# 조사 진술과 보수적 처분 BFF

이 경로는 사람이 조사 기록을 남기고, 기존 UNKNOWN 작업을 `UNRESOLVED / QUARANTINED`로 처분하는 Core 흐름에 연결한다. 진술 저장은 작업 Outcome을 변경하지 않는다. 처분은 실행·재시도·자원 해제를 허가하지 않는다. 원래 작업·허가·근거·영향 셀·서명된 절차·정책 generation의 검사는 Core가 소유한다.

## HTTP 입력과 응답

| Method/path | 입력 | 응답 |
|---|---|---|
| GET `/api/v1/investigation-context` | `operation` | `{context,context_digest,operation_authorized:false,resource_release_authorized:false}` |
| POST `/api/v1/investigation-attestations` | `{request_key,command:AttestSubmit}` | 진술 응답 |
| GET `/api/v1/investigation-attestation` | `operation,id` | 진술 응답 |
| GET `/api/v1/investigation-attestations` | `operation`, 선택 `after`, 필수 `limit`(1..50) | `{items:[진술 응답],next,operation_authorized:false,resource_release_authorized:false}` |
| POST `/api/v1/recovery-dispositions` | `{request_key,command:RecordDisposition}` | 처분 응답 |
| GET `/api/v1/recovery-disposition` | `operation,id` | 처분 응답 |
| GET `/api/v1/recovery-dispositions` | `operation`, 선택 `after`, 필수 `limit`(1..50) | `{items:[처분 응답],next,operation_authorized:false,resource_release_authorized:false}` |

진술 응답은 `{attestation,reference,current,outcome_changed:false,operation_authorized:false,resource_release_authorized:false}`다. 처분 응답은 `{receipt,reference,current,operation_authorized:false,resource_release_authorized:false}`다. `reference`와 `context_digest`는 서버에서 계산한다. `current`는 Core View의 현재 원작업·근거·범위·정책 cut 일치 여부를 그대로 표시하며, 승인이나 운전 가능 의미로 바꾸지 않는다. 원본 record의 reference와 과거 actor/session/terminal은 변경하지 않는다.

`AttestSubmit`은 `id,operation,expected_operation_revision,context_digest,procedure_digest,evidence_ids,assertion,note,occurred_at`만 받는다. assertion은 `RESULT_REMAINS_UNKNOWN`, occurred_at은 사람이 보고한 UTC RFC3339 문자열이다. BOOTTIME이나 현재 관측의 freshness로 사용하지 않으며 recorded_at은 Core의 P clock이다. note에서 성공·실패·PASS 또는 작업 결론을 파싱하지 않는다.

`RecordDisposition`은 `operation,expected_revision,evidence_ids,procedure_digest,disposition,reason`만 받으며 disposition=`QUARANTINED`, reason=`UNKNOWN_OUTCOME`이다. caller의 actor/role/session/terminal/URI/path/certificate/PASS/Outcome 필드를 허용하지 않는다. snapshots, Ticket/Prepared, worker 정책·절차 bytes를 HTTP body로 받지 않는다. 모든 입력은 기존 domain ID/Digest/Counter 및 중복/unknown field 거부 decoder를 사용한다.

## 인증·검증·과거 응답 회수

기존 terminal mTLS, cookie/인증서 결합, Host/Origin/CSRF/JSON Content-Type 경계를 그대로 적용한다. HTTP는 매 요청 현재 writer UserProfile에서 RecoveryLead, `procedure::can_report`, 등록 단말을 확인한다. Host/Executor/OperatorApi 역할이 섞인 서비스 신원도 이 인간 보고 경로를 사용할 수 없다. Core는 이어 현재 원작업·영향 closure·저장된 원 scope 접근권을 preflight와 commit에서 다시 검사한다.

제출 순서는 **현재 인간 인증 → Core preflight → 필요한 경우 bounded worker → Core commit → 현재 actor로 Core View 조회**다. preflight의 Recorded는 worker 없이 회수한다. Verify일 때만 배포 설정의 investigation worker를 선택하며, 없으면 `503 INVESTIGATION_NOT_CONFIGURED`다. preflight가 이미 거부한 요청을 미구성 worker 오류로 바꾸지 않는다. worker 검증 실패는 경로·서명·파일 오류 원문을 숨긴 `409 INVESTIGATION_VERIFICATION_FAILED`다. writer 오류와 outcome_unknown의 기존 mapping은 보존한다.

진술과 처분의 idempotency 의미는 Core와 같다. 원래 key로 과거 응답을 회수할 때 expected revision은 과거 CAS로 보존하고 evidence_ids는 집합 의미로 비교한다. 새 transaction의 CAS 검사는 Core가 수행한다. 저장된 actor principal과 의미상 요청 필드는 대조하지만, 재로그인 후 historical session/terminal이 현재와 같아야 한다고 요구하지 않는다. HTTP 재조회한 record reference가 preflight/commit의 원본과 다르면 거부한다.

모든 응답의 authority flag를 검사한다. 처분 receipt는 `resources_released:false`이고 after.operation이 `UNRESOLVED / QUARANTINED`인 경우만 반환한다. 잘못 연결된 worker/runtime 응답은 sanitized 오류로 처리한다. 새 `TerminalHttps::new_with_investigation`은 optional worker를 추가하며 기존 생성자들은 None으로 위임한다.

## 시험 범위

`tests/investigation.rs`는 실제 단말 TLS·SQLite writer로 로그인, 현재 역할, 역할 회수와 서비스 신원 거부를 확인하는 시험 소스다. investigation Command의 조회/Recorded 응답만 명시적 routing fixture로 대체하여 strict DTO·CSRF·pagination·기존 key 전달·과거 응답 회수·서버 digest·authority flag 거부를 시험한다. private Ticket/Prepared를 합성하지 않는다. 이 fixture의 UNKNOWN/처분 원문은 Core T5 검증이나 서명 절차 수행의 인수 증거가 아니다. 실제 worker 파일 pin/서명 검증과 Core UNKNOWN→UNRESOLVED transaction은 별도의 Core/runtime 시험과 제품 인수에서 검증한다.
