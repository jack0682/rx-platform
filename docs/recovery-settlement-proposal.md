# Proposed: finishing admitted work after an executor incarnation change

Status: **CANDIDATE IMPLEMENTED — not published or physically qualified.**

The worktree now contains the bounded current-authority settlement transition and
its candidate contract under `spec/settlement/v1`. The alternatives below record
the design decision; they are not a claim that the wider recovery goal is done.
Known finite work with a retained Host is the first scope. Changed Host/native
lineage, unknown effects and fresh production authority remain separate work.

Public testing exposed a sequencing defect: executor replacement first fenced
the cell, but a later Host qualification poll suspended the prior active batch
and advanced its epoch again. A recovery authorization issued between those
transactions correctly failed its current-epoch check. Executor replacement now
suspends affected pending/active qualification batches in the same transaction
that retires the old peer. A late poll cannot repeat that known invalidation.
No epoch check, fence requirement or ordinary release guard is relaxed. A
deterministic regression covers this boundary and same-incarnation reconnect.

The follow-on requalification path also needs provenance for the first
restriction created by executor replacement. Within that same transaction, RX
now records Change ownership only for new restrictions on affected cells whose
exact active qualification, configuration and runtime match the retiring batch.
The pre-invalidation block ID set prevents adoption of older holds or unrelated
authority restrictions, even when their reason names match. This reuses the
existing internal Change/block ownership format; it changes no public wire
schema. Historical unowned restrictions are not retroactively assigned.

Ownership is not clearance. Fresh requalification evidence, separate review and
issuance, exact clearance IDs, current Host qualification exchange and explicit
activation are still required. Settlement never performs those steps. Negative
tests retain a prior manual hold and an unrelated AuthorityRevoked restriction.

This decision record is grounded in a public P/H/installed-rxclpy reproduction using two external digital providers. It is not permission to bypass the existing release guard, to restore a retired session, or to promote a read-only recovery case to task success.

## Observed problem

A normal job and a lost admission reply can reach `Succeeded / Valid / Released`, a confirmed-completed part, and a completed finite-budget run. An actual executor peer incarnation change retires its previous session and invalidates the cell closure (`engine/executor_peer.rs`). The original admission receipt can still be recovered with the original request key/command under the authenticated new session (`executor_requests.rs::executor_submit`), because the prior request check precedes new-work admission.

The original effect then remains known, while its resource disposition is quarantined and its run is recovery-required. Normal release (`handover.rs::release_resources`) requires `ready(cell)` and original permit epoch/scopes/Host boot continuity. Merely observing a successful effect or issuing `Operation.Reconcile` cannot satisfy those requirements.

Related existing mechanisms have narrower scopes:

- Non-operating case closure deliberately retains unresolved work and resource obligations.
- Host recovery bindings preserve pinned reads and fencing for a Platform runtime restart with a retained Host baseline. Their context builder requires a runtime restart origin and retained Host identities; they are not a generic executor/Host restart or task-completion mechanism.
- `complete_part_transition` checks actual process frontier or succeeded/valid/released operations. Reuse these semantic checks; do not set a local consumer Done flag instead.

## Required end state

For each supported positive recovery case, the requested effect must be established, all job attempts and resource obligations must be resolved, and the original successful part/run must reach their declared terminal state. A later job uses fresh authority. Original execution authority must never be reinstated.

`UNKNOWN`, retained quarantine, a closed diagnostic case, or an abandoned original run alone do not satisfy that positive requirement. Authoritative no-effect recovery is a separate obligation; `lookup(None)` cannot establish it.

## Options that must be compared

### A. Existing API orchestration only

Keep immutable request identity, rebind only authenticated transport context when needed, recover the receipt, query, and request reconciliation. This already solves the same-incarnation reply-loss case. It does not close the reproduced new-incarnation quarantine. Keep this as the minimal baseline; do not build another request manager around functions that already exist.

### B. Explicit current, non-actuating settlement authority

Add a supported path whose authority permits only accepting correlated historical results, confirming current handover, and closing already-admitted work. It must not issue an execution permit, Arm, Submit, alter the original outcome, or clear unrelated blockers. Reuse existing ledger transactions, role checks, pinned Host transport, current read evidence, and immutable run configuration.

This is the conservative candidate for the reproduced closure gap. A concrete design must pin all of:

- original operation/invocation/intent/profile, run/part, original evidence and permit identity;
- current installation/store/runtime/clock and affected cell/scope revisions;
- current authenticated Host identity, journal identity, native lineage, and complete relevant scope;
- confirmed current fencing of old effect-capable requests;
- current resource holder and fence values, rather than only a list of resource names;
- fresh correlated no-pending/control/support observations from the pinned producer;
- current approval actor, scope, revision, expiry and idempotent request identity.

UI-supplied booleans or deserialized source records are not verified reads. Existing `VerifiedRead` patterns are useful, but the specialized Host recovery context must not simply have its blockers removed to fit this scenario.

First establish a full positive/negative design for already-settled, valid work and a retained Host. Explicitly leave changed Host/native lineage and authoritative no-effect as unimplemented until their evidence path is designed. This is an implementation sequence, not a narrowing of final acceptance.

A closure transaction must preserve a revoked mandate as revoked; reusing normal part completion must not accidentally rewrite old authorization history. Completing a part/run must not implicitly requalify a cell or authorize the next job.

### C. Reconsider executor-lifecycle coupling

An executor process is not necessarily the authority that supervises an already-admitted native action. For independently supervised finite actions, retiring an executor session may not need to invalidate every equipment qualification. However, `FiniteAction` alone does not prove independence: an action can rely on executor-based perception, monitoring or supervision.

Do not unconditionally remove `invalidate_closure`. Before adopting a narrower invalidation policy, define the supervision dependency, default to existing strict behavior, retire old session/mandate rights, and prove that old queued work, local protection, shared resources and overlapping tasks remain constrained. A supported run-reattachment protocol would still be needed. Compare the lifetime and authoring cost of this option against explicit settlement instead of protecting the current architecture by default.

## Rejected shortcuts

- Assigning the old boot/session/epoch to the new process.
- Modifying the old permit to match current epochs.
- Removing readiness/continuity checks from ordinary release.
- Writing directly into P resource/run rows or treating consumer state as authoritative.
- Adding a provider-name exception for the file/SQLite example.
- Reissuing a new operation while the original effect is uncertain.
- Treating a new source generation as proof of absence.
- Calling an original record the effect of a different logical job.

## Required counterexamples before implementation is accepted

1. Revoked, foreign or insufficiently scoped recovery actor; original executor cannot self-approve an elevated recovery action.
2. Old proposal after any relevant epoch/configuration/Host/native generation or resource holder change.
3. Wrong operation/invocation/profile, stale or conflicting handover observation, absent residual-command proof.
4. Response loss at proposal/approval/closure commits; duplicate request must recover the same decision.
5. Process crash around closure; operation/resource/part/run state must not diverge across partial commits.
6. Late contradictory evidence after a previous conclusion.
7. A different task holding the same named resource.
8. No-effect claim derived only from a missing lookup; continued quarantine is required.
9. Current safety or supervision dependency unavailable; fresh data cannot be fabricated by changing the policy.
10. Normal release behavior and its refusal tests remain intact.

## Separate input-binding gap

The public next-job probe prepared a valid new parameter artifact and a new logical job, but current `dispatch.rs` requires the whole configured intent digest to match. Existing reviewed configuration/package change must be assessed before introducing a late-binding contract. Removing job-ID uniqueness or weakening intent authorization is not an acceptable workaround. Record the number of authoring/review steps required to change only job data; correctness and usable framework scope are separate questions.

## Implementation entry points

- current actor and invalidation: `engine/executor_peer.rs`, `engine/invalidation.rs`;
- receipt recovery: `engine/executor_requests.rs`, gRPC `cell_negotiation.rs`;
- handover/resource transaction and part completion: `engine/handover.rs`;
- current pinned/fenced recovery read model: `host_recovery.rs`, `engine/host_recovery/`;
- procedure/case policies: `intervention.rs`, `procedure.rs`, `closure.rs`;
- runtime/transport integration: `rx-runtime`, `rx-host-client`, `rx-api`;
- any protocol change: canonical contracts first, then generated protocol/shared SDK copies and compatibility checks.

The next implementation must choose and justify a concrete current-authority/evidence path. This proposal alone is neither a recovery capability nor a completed final goal.
