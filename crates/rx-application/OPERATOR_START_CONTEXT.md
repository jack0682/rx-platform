# 작업 시작 후보·시작 시도 조회

`GET /api/v1/run/start-context?cell=...&run=...&purpose=PRODUCTION&budget_limit=2`는 현재 Operator와 셀 접근권을 확인한 뒤 같은 transaction에서 현재 revision·구성·envelope·최대 수량·Run과 실제 StartRun 후보를 반환한다. `budget_limit`는 Counter 문자열이며 목적에 따라 budget unit은 PRODUCTION/PART_ATTEMPT 또는 SETUP/OPERATION_COUNT로 정한다. 현재 Run에 이미 budget이 있으면 기존 값을 보여 주고 다른 후보는 거부 사유를 반환한다.

`can_request`는 `checked_at` 당시의 후보 평가이며 permit나 시작 완료가 아니다. 유효 Operator 세션에 등록 단말이 없으면 조회는 `blocking_reason=FORBIDDEN`으로 설명할 수 있지만, 권한 없는 셀이나 폐기된 단말의 업무 본문은 반환하지 않는다. 나머지 실행 가능성은 현재 자격·조건·Host·Executor 세션과 실제 StartRun의 검사에 따른다. 불명확한 내부 상태를 임의의 정상 값으로 바꾸지 않는다.

Engine의 `operator_start::validate_candidate`는 읽기 전용 공통 검사다. 기존 StartRun은 현재 사용자/단말 권한 확인과 원래 key 회수 순서를 유지한 뒤 이 검사를 호출한다. 이후 mutation에서만 budget·attempt·Arm outbox를 기록한다. context 조회에서는 원장·예산·attempt·자격을 생성하지 않는다. 별도 PrepareStart 권한/토큰은 없다. UI는 정확한 반환 후보를 확인창에서 고정하고, 실제 송신 시 기존 StartRun이 CAS와 조건을 다시 검사하게 한다.

`GET /api/v1/run/start-attempt?cell=...&run=...&id=...`는 기존 셀 읽기 접근권과 정확한 cell/run/attempt 상관관계를 확인하고 현재 Run과 저장 StartAttempt를 반환한다. `deadline_status`는 WITHIN_DEADLINE/ELAPSED/CLOCK_CHANGED이며 저장 status와 별개다. ELAPSED 조회가 ARMING을 REJECTED로 바꾸거나 STARTED를 취소하지 않는다. 모든 실제 Host ACK 뒤 P가 기록한 STARTED/EXECUTING만 시작 확정이다.

후속 상주 셀 executor service는 Run이 없는 idle에서 실제 세션을 열고 유지해야 한다. StartRun 이후 assignment 조회에서 ARMING/EXECUTING Run에 연결하는 경로를 목표로 하며, 특정 Run의 사전 배정이나 브라우저의 Executor credential을 요구하지 않는다. 현재 구현한 것은 조회 API와 그 검사이며 상주 service의 실제 연결은 후속이다. Host producer 재연결·전체 복구·만료 attempt의 명시적 조정은 별도 구현 범위다. 현재 기한 경과만으로 새로운 시작을 자동 재시도하지 않는다.

작성한 시험은 readonly candidate와 명시 mutation의 분리, 실제 두 Host ACK, 예산/envelope/unit 변조·stale revision, 단말 없는 Operator와 셀/설치 접근권, 응답 유실 후 동일 attempt 회수, 기한 조회의 무변경, 직접 mTLS HTTP 조회와 단말 폐기를 다룬다. SIMULATION fixture는 운전 소프트웨어 경계 시험이며 첫 물리 셀의 commissioning을 만들지 않는다.
