# Platform Host client

This crate converts application DTOs into the frozen RX wire contract and communicates over mTLS. It does not modify the application store itself.

The application first commits an outbox entry, claims emission, sends the request with that stable key, and records the Host receipt in a new atomic transaction. A PREPARED receipt schedules Authorize; a captured Host result alone does not set a platform outcome. Native evidence is ingested through T2 and the configured completion policy.

Run the cross-repository, separate-process integration test with:

    ./tools/test_host_e2e.sh

The script builds a feature-gated simulation Host fixture from rx-solutions and then runs the ignored network integration test with its exact path. The regular workspace test command intentionally does not silently substitute a mock for that process.

The client includes bounded outbox dispatch, existing-invocation reconciliation and durable query-driven resource handover. `Operation.Reconcile` schedules Host reads; it never directly resubmits a production invocation. Three fresh correlated handover facts and the existing application release transaction are required. False observations remain recorded without releasing resources. See [query and handover semantics](../rx-application/RECONCILIATION.md).

Background Evidence.Publish is owned by the solutions Host. Product supervision, strict control-delivery latency, indexed large-journal scheduling and the complete recovery workflow remain pending.

새 Host process-configuration client는 Inspect/Apply/Lookup과 strict payload/identity 검사를 제공한다. 호출 전에 요청을 영속 보관하고 오류 후 같은 요청을 조회해야 한다. [Host 계약·한계](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/PROCESS_CONFIGURATION.md)를 따르며 [P 변경 원장의 영속 coordinator](../rx-application/HOST_CONFIGURATION_DISPATCH.md)가 명시적 Batch 요청·전송 전 commit·같은 ID 조회·receipt 보존을 연결한다. P 구성 교체·재검증 검토와 [전역 자격 활성화](../rx-application/QUALIFICATION_ACTIVATION.md)를 연결했다.

선택 qualification client는 [Host 자격 수용](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/QUALIFICATION_ACCEPTANCE.md)의 Inspect/Accept/Lookup을 제공한다. 이 raw transport는 P의 발급 권위나 영속 task를 대체하지 않는다.

`qualification_worker`는 P 영속 발급 task를 수행한다. Arm의 block 해제는 P가 승인한 ID에 한정하고 현재 Host block을 확인한 뒤 전달한다. 동일 lease의 갱신과 자격·Arm을 구별한다.

P-only 재시작 뒤 관리자 승인에 따른 [Host 복구 통신](HOST_RECOVERY.md)을 제공한다. 현재 transport/source와 최초 불변 baseline을 확인하고 기존 Fence·receipt·native 결과만 회수한다. RECOVERY_ONLY는 운전용 등록·자격·Arm·Run 재개 허가가 아니다.
