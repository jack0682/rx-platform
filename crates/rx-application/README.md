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

[Package intake](PACKAGE_INTAKE.md) binds a separate worker's file verification/storage result to the current user/cell/policy context and records it in the ledger. The result is AWAITING_REVIEW, not approval or activation.

[Process review and software approval](PROCESS_REVIEW.md) compare actual verification material signatures and source/result/cell context, and bind a decision by a Verifier different from the submitter to a version. Activation and physical qualification are separate.

[Approved-process change planning](PROCESS_CHANGE.md) implements before/after, impact review, STAGED and explicit fence preparation. Host configuration acknowledgments and replacement of the actual installed selection are subsequent work.
