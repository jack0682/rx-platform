# Host 복구 통신 HTTP 경계

`host_recovery.rs`는 `rx-runtime::host_recovery::Service`에 연결하는 BFF다. 이 경로의 성공은 RecoveryOnly 통신이며 grant·Arm·operation admission·native replay 허가가 아니다. 제품 Host client worker는 기동 설정의 고정된 transport registry를 사용하고 실제 읽기를 writer에 제출한다. HTTP 입력에서 snapshot, URI, certificate, path, RPC method, principal/role/session을 받지 않는다.

## 입력과 응답

| Method/path | 엄격한 입력 | 응답 |
|---|---|---|
| GET `/api/v1/host-recovery-context` | query `host`, `origin` | `{context, context_digest, expected_cells}` |
| GET `/api/v1/host-recoveries` | query `host`, 선택 `after`, 필수 `limit`(1..50) | `{items:[{view,proposal_digest}],next}` |
| POST `/api/v1/host-recoveries` | `{request_key, command:{host,origin,expected_context,expected_cells}}` | `{view, proposal_digest}` |
| GET `/api/v1/host-recovery` | query `id` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/approve` | `{request_key, command:{id,expected_revision,proposal_digest,expected_cells}}` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/progress` | `{id}` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/query` | `{id,operation}` | typed `QueryResult` |

`context_digest`와 `expected_cells`는 서버의 `Context` 메서드에서, `proposal_digest`는 저장된 `Binding::proposal_digest()`에서 계산한다. UI는 JCS나 digest 의미를 재구현하지 않는다. mutation envelope와 모든 입력은 unknown/duplicate 필드를 거부하며 ID/Digest/Counter는 기존 domain JSON 형식을 사용한다. 변경 CAS와 idempotency 순서의 최종 책임은 writer에 있다.

`Binding.requested_context_digest`는 원래 Prepare.expected_context를 보존한 불변 요청 상관값이다. 제안 준비 중 발견한 blocker가 실제 `binding.context`에 추가되면 그 진단 context의 digest는 원래 요청과 달라질 수 있다. Propose 응답은 HTTP 서버가 원래 요청 digest·Host/origin·셀 revision 집합·현재 사용자의 principal을 대조한 뒤 반환한다. 저장된 proposed_by.session/terminal은 제안 당시의 역사적 식별자이므로 같은 principal이 재로그인하거나 등록 단말을 바꿔 원래 key로 회수할 때 현재 값과 같을 필요가 없다. 현재 인증·전체 셀 접근권은 HTTP와 core가 다시 확인하며 저장된 식별자를 현재 권한으로 승격하지 않는다. UI는 blocker를 제거하거나 context digest를 재계산하지 않고 서버가 준 opaque `proposal_digest`로 같은 제안을 검토한다.

목록은 다른 ReleaseManager나 새 브라우저가 저장된 복구 기록을 찾는 조회이며 로컬 저장 UUID에 의존하지 않는다. 현재 actor 인증을 먼저 확인하고 1..50 크기 제한을 검사한 뒤 service를 선택한다. core의 접근권 필터와 원래 next cursor를 보존하며 목록 읽기로 소유권/승인/Fence를 만들지 않는다.

propose/approve는 원래 request_key를 그대로 전달한다. 응답 유실 뒤 같은 내용을 같은 key로 회수한다. progress는 이미 승인된 binding의 기존 Fence task/request ID만 이어 처리하며 승인이나 새 Host key를 생성하지 않는다. query는 선택한 기존 operation의 결과 조회/원래 receipt 회수다. operation을 다시 실행하는 동작으로 바꾸지 않는다.

`QueryResult.lookup`은 `NOT_NEEDED`, `PREFIX_OBSERVED`, `UNAVAILABLE`, `UNSUPPORTED`다. evidence는 선택 operation과 상관된 진단 자료이며 `evidence_complete:false`, `operation_authorized:false`다. 원래 Host publisher가 정확한 전체 prefix를 전달해야 한다. HTTP는 View의 operation_authorized=true 또는 QueryResult의 operation_authorized/evidence_complete=true를 잘못 연결된 서비스 응답으로 거부한다.

## 인증·가용성

기존 direct terminal mTLS, Host, Origin, cookie/terminal 결합, CSRF와 JSON Content-Type 검사를 그대로 사용한다. 각 handler는 현재 writer의 UserProfile을 읽어 ReleaseManager 역할과 등록 단말을 확인한 뒤 worker 가용성을 공개한다. worker/core는 실제 I/O와 authoritative transaction에서 현재 세션·역할·단말·설치·전체 Host cell 범위를 다시 검사한다. HTTP preflight 자체를 재사용 가능한 권한으로 취급하지 않는다. loopback 개발 router에는 worker를 구성하지 않으며 등록 단말을 합성하지 않는다.

인증 없는 요청은 401, 금지 역할/단말은 403이다. 유효한 ReleaseManager에게 미구성 worker는 503 `HOST_RECOVERY_NOT_CONFIGURED`를 반환한다. `WriterError`는 기존 API mapping을 보존하며 writer 응답 유실의 outcome_unknown=true를 숨기지 않는다. worker transport 오류는 503 `HOST_RECOVERY_UNAVAILABLE`(outcome_unknown=true), 검증 불가 read는 409 `HOST_RECOVERY_INVALID_READ`, worker 포화는 429 `HOST_RECOVERY_BUSY`로 제한한다. 원본 transport 오류문·인증서·경로·내부 RPC 내용은 반환하지 않는다. 네트워크 유실 후 승인된 바인딩이 FENCING 상태로 반환되는 것은 정상 복구 진행 응답이며 활성화 완료가 아니다.

`TerminalHttps::new_with_host_recovery`는 package worker, optional S operator bundle, optional recovery service를 함께 받는다. 기존 `new`, `new_with_package_intake`, `new_with_operator_ui` 생성자는 recovery=None으로 위임해 기존 호출자를 유지한다.

## 검증 범위

`crates/rx-api/tests/host_recovery.rs`는 실제 terminal TLS/SQLite writer로 인증, 재로그인·등록 단말 변경 뒤 과거 응답 회수, 현재 역할 회수, 미구성 서비스 응답, 엄격한 입력, CSRF, 원본 ID/key/CAS 전달, writer 오류 보존과 서버 digest 출력을 확인하는 시험 소스다. RoutingMock은 전달 오류와 출력 DTO 시험 자료만 내며 Host 복구 성공/권한을 합성하지 않는다. 실제 pinned Host worker·Fence·복구 binding/조회 검증은 제품 worker/core 시험과 별도 인수에서 수행한다. 이 HTTP 시험만으로 복구 또는 운전이 인수됐다고 주장하지 않는다.
