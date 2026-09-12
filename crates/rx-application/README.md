# Application transactions

This crate owns installation/cell/run/activation/operation state transitions over rx-ports. It imports neither SQLite nor Protobuf nor ROS.

- identity/access: stored principal/session/terminal checks. Identity is constructed by a credential-verifying adapter and is not deserializable from a request body.
- configuration/admission: immutable cell binding, qualification cache, Host registration, condition and grant checks.
- workflow: create a prepared run, assign its immutable budget at StartRun, collect all Host Arm acknowledgments, count part attempts and resolve stable activations.
- dispatch: bind an exact resolved intent to the run/node/slot, reserve globally named resources, persist operation/permit/outbox/key together.
- observation/invalidation: preserve contradictory evidence and generation changes even when returning an error; revoke mandates, void unsent work, fence affected cells and retain resource quarantine.

Current scope is the finite dependency-step binding. Each node has one occurrence per part; its visit is the part ordinal across the entire run. Source-level branch/loop/subflow compilation and general checkpoint handling are subsequent work, not replaced by this temporary binding.

QualificationAuthority is an immutable in-memory verification catalog prepared outside the writer. It is not a network/file verifier and is never a caller-provided PASS flag. The test authority recognizes only synthetic simulation evidence.

Current grants and Host Arm receipts are supplied by test fixtures/receipt adapters. Host journals, actual native dispatch and completion evidence ingestion are not implemented in this crate yet. This is not a public HTTP/RPC server or hardware commissioning result.

[패키지 반입 접수](PACKAGE_INTAKE.md)는 별도 작업자의 파일 검증·보관 결과를 현재 사용자·셀·정책 문맥에 결합해 원장에 남긴다. 결과는 AWAITING_REVIEW이며 승인·활성화가 아니다.

[공정 검토·소프트웨어 승인](PROCESS_REVIEW.md)은 실제 검증 자료의 서명과 원문/결과/셀 문맥을 대조하고, 제출자와 다른 Verifier의 결정을 버전에 결합한다. 활성화·실물 qualification은 별도다.

[승인 공정 변경 계획](PROCESS_CHANGE.md)은 before/after·영향 검토·STAGED·명시적 fence 준비를 구현한다. Host 구성 ack와 실제 설치 선택 교체는 후속이다.
