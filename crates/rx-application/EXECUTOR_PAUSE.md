# Executor PauseRun and authority revocation

Status: connected to actual Workflow.PauseRun, S's durable pause request and the C++ halt fixture. A Pause response is not completion of native cancellation/stop/access authorization/resource handover.

## Scope

Current Host permits/Arm/fences bind to cell epoch and scope. Changing only local Run.state when pausing a run already executing or preparing Arm would leave cleanup of existing Host authority unclear. This implementation therefore revokes the relevant cell together with its validated shared scope/resource closure.

- An executing origin run becomes PAUSED; other executing/preparing runs in the affected closure become RECOVERY_REQUIRED.
- Cell/scope epochs increment and an ExecutorPause LATCHED block is recorded. Active mandates and unused permits are revoked, with fence outbox records written together.
- A PREPARED run without Arm/mandate/permit changes only its own state to PAUSED. Other cell work is not unnecessarily fenced.
- A run already PAUSED/RECOVERY_REQUIRED/COMPLETED/ABANDONED does not create a new fence or downgrade state. A completed state is not overwritten with PAUSED.
- Budget and part/activation/operation identities are preserved. There is no automatic restart or budget refund.

This Pause is a restricted path for explicit halt/pause requests. Normal WAIT_TARGET or temporary read delays are not converted to this API; those states follow existing transient/continuity rules. Actual unpausing and run restart require separate explicit conditions/reconfirmation procedures, and the complete restart API is not yet finished.

## Checks and transaction

After checking current executor mTLS session/account role/cell definition negotiation/assignment, compare the key and complete PauseRun payload, including expected run revision. New requests check run CAS; an EXECUTING run must match the current owner session. The same key/body returns the original response.

State/epoch/block/mandate/permit/outbox and control event/checkpoint are handled in one transaction. A separate second Run update is not made to set the origin PAUSED. Existing invalidation is parameterized to capture the final state of the same commit.

`Finalized` can read the completed projection in the same transaction after control capture and store only the request result. It exposes no core put/append/outbox mutation capabilities. This caches the PauseRun response at the same cut as the final RunSnapshot, without mixing later state into the initial response through read-after-commit.

## Delivery state and results

- NEW operation deliveries are sealed as VOIDED. NOT_EXECUTED evidence is recorded only when P can prove no native emission.
- If already EMIT_ENTERED, Pause alone does not create CANCELED/NOT_EXECUTED/success. Unresolved delivery remains in UNKNOWN/quarantine and reconciliation paths.
- Issued permits become VOIDED, so a late PREPARED receipt cannot create a new Authorize outbox item.
- Pending start attempts become REJECTED, and late Arm acknowledgments cannot restart the run. An unissued Arm cannot pass the current-attempt check to emit either.
- Late correlated native results remain acceptable. Even after an actual result is established, resource handover is separate; quarantine/support is not arbitrarily released.

It is not recorded as having called fieldbus stop or robot cancel on their behalf. Fence delivery, investigation/cancellation of already-entered operations and local protection retain their respective contracts.

## S requests and repeated halts

The S journal records the PauseRun body and complete frozen RunView response. A pause logical key includes the original executor session/epoch as well as the same run/visit/root. The canonical representation of existing operation keys without this optional control identity is preserved.

A lost response for the same pause is therefore recovered with the same key, while a new halt in a different, subsequently explicitly authorized context becomes a separate request. A historical halt does not automatically pause execution in a newer epoch. If a new snapshot shows an already restricted run, this is recorded as an observation without fabricating an unreceived RPC reply.

The S worker validates the PauseExecutor request created by C++ `Executor::halt` and forwards it through this path. After the RPC response, C++/S does not independently declare resumed operation or physical stop completion.

## Validation

- Atomic RunSnapshot/fence/invalidation/key results on failure immediately before core commit and lost response after commit.
- Shared-cell closure, one origin Run revision increment, part/budget preservation and immutable same-key responses.
- Sealing unissued operations, no Authorize after late PREPARED, and rejection of late acknowledgments after pausing during Arm preparation.
- Already-emitted requests are not presumed canceled; subsequent correlated results are accepted and recorded.
- A PREPARED/no-Arm run does not change the cell epoch.
- Actual mTLS PauseRun and C++ halt → S journal → P handling, including pause key/unreceived-state preservation after a lost pause response and new-boot recovery.

Test clocks/qualification/Hosts/operators are explicit simulation fixtures. Actual device stopping time, safe access and field recovery acceptance are separate and are not established by these passing tests.
