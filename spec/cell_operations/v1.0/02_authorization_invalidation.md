# Start Intent, Dispatch Authorization, and Invalidation

Normative: RX Cell Operations Contract v1.0 · P=platform, H=Host. Reuses common v1.0 single-writer, key, epoch, journal, and native-dispatch gate rules.

## 1. Independent state

CellContext contains `revision, cell_epoch, scope_epochs, active_envelope, qualification, desired_mode, acknowledged_hosts, block_reasons, open_cases`. desired_mode is `SETUP / AUTOMATIC / RECOVERY / MAINTENANCE`, distinct from observed actual native modes. Changing a mode name alone does not start native motion.

cell_epoch is the overall boundary for cell definition/active configuration/Runtime generation. scope_epochs is a map of generations per affected zone registered in CellDefinition. Local invalidation increments only scope epochs in the impact closure; changes to overall meaning, such as cell-configuration replacement/Runtime restoration, update cell_epoch and all scopes. Each operation/mandate binds to its complete dependency scope vector. A local incident does not automatically stop other runs/scopes whose dependency relationships have been established.

Qualification may be QUALIFIED while block_reasons exist, and a Run may be EXECUTING while its next operation waits. `READY` is only a current derived UI indication, not authority that may be stored and reused indefinitely. Clearing a block is also separate from restoring a RunMandate.

## 2. RunMandate

A RunMandate is durable start intent bound to **a specific run/recipe/envelope, purpose and quantity or finite work scope, cell_epoch and scope vector, issuing actor, and start action**. States are `ACTIVE / REVOKED / EXHAUSTED / CLOSED`; a REVOKED ID never returns to ACTIVE.

Initial creation checks valid qualification, matching cell/recipe configuration, applicable mode/role, absence of related open intervention cases, PASS for required start conditions, current executor session, and an explicit start request. RestartRun may atomically consume/close only designated READY_FOR_RESTART cases/clearances with the new mandate commit. Other open cases/external restrictions are not exceptions and block restart. Distinguish condition checks, Host preparation, mandate commit, and native dispatch.

Restart preparation first installs a new scope epoch through `PrepareRestart`, fencing old production/recovery permits, then rechecks entered native work, residual queues, support, and current conditions. **In that new epoch, record case transitions to READY_FOR_RESTART and clearance issuance in the same transaction**, with the post-transition case revision set in the clearance. New recovery operations are also rejected in this state. If more work is needed, void the clearance and return to REVALIDATING/PROCEDURE_ACTIVE.

Start/restart is accepted as a StartAttempt, followed by collection of `ArmCell` acks from related Hosts. RestartRun uses the same epoch/clearance created by PrepareRestart without incrementing the epoch again. ArmCell records readiness to clear only the explicitly named RX-gate blocks; it performs no physical reset/output. Only after all Hosts are prepared at matching epochs and the attempt's cell/case revisions remain valid does P atomically commit the new mandate, run start, clearance consumption, and target-case closure. Invalidation during preparation rejects the attempt and retains protection/blocks. Do not clear unrelated-case blocks with it.

Production-start sources are `OPERATOR` or `BATCH_SCHEDULER` explicitly permitted by the envelope. Scheduler authority covers only specified orders, quantities, and recipes. After human intervention/safety stop/recovery, `RestartRun` requires new start intent from an OPERATOR with recovery responsibility; schedulers cannot substitute. Distinguish initial start from recovery restart through separate APIs/permissions.

Budget units are explicitly `PART_ATTEMPT / OPERATION_COUNT`. Material-production runs use PART_ATTEMPT, consuming one unit when P first persistently creates a unique part_attempt_id. Multiple activations such as picking, feeding, and doors within that material attempt share the same consumption. SETUP runs use OPERATION_COUNT, consuming one unit on new-operation creation in T1. Looking up the same key/ID never consumes another unit.

Persist the run budget ledger separately from mandates. Creating a new mandate after restart must not reset prior consumption to 0. Unknown outcomes, defective parts, and cancellation before dispatch do not erase attempt history or automatically refund budget. Explicit continuation of the same part_attempt is not counted as a new material attempt; a new material-processing attempt requires a new ID and remaining budget. Target good-part quantity differs from permitted attempt budget. This revision keeps a run's limit immutable. If more attempts are needed, an operator specifies additional scope through a new run and start intent referencing the previous run/disposition. A new run also does not remove related unresolved/material/scope restrictions or authorize repeating uncertain operations. A new human approval is not required for every internal node.

Budget consumed=limit means **no new part/operation creation**. It alone does not block remaining nodes of an already-counted part or dispatch/result checks of already-created operations. Record mandate EXHAUSTED when it cannot create new budget units and all permitted existing-attempt execution/explicit continuations have finished. Even with 0 remaining budget, restart may create a new mandate containing only explicit continuation of existing parts. Additional part creation remains rejected.

Closing a UI window/disconnecting its display does not automatically revoke a production mandate. Runtime/Host/executor validity and site supervision/responsible-person conditions are separate. Envelopes requiring those conditions must specify their sources/loss policies; a UI heartbeat does not prove a person's physical presence.

## 3. Transient holds and start-intent revocation

| Event | New normal operations | Mandate/next action |
|---|---|---|
| Normal recipe WAIT_TARGET not yet reached | Wait only for the related next operation | Remain ACTIVE; continue within original scope once target is reached |
| Temporary absence of an admission observation not yet associated with native effects | Hold related dispatch | If state/authority/configuration are unchanged and evidence recovers, a new permit may be evaluated |
| Lost RPC response while querying the same operation's outcome | Hold related subsequent dispatch | Query the same key/ID. No automatic repeat call. Original run may continue when correlated results/current conditions recover |
| Source contradiction, DISPUTED result, unknown physical change | Hold impact closure; perform required local reaction | Revoke mandate; require investigation/intervention case and restart |
| Safety/emergency stop or unclassified stop | Block new production in affected scopes | Revoke mandate. New RestartRun after function-specific reset/rechecks |
| Operator Pause/Stop, human access/manual manipulation | Block new production in scope | Revoke mandate. Pause alone does not establish physical stopping/access |
| Controller/Host/P/executor restart or command-owner change | Block related new production | Invalidate existing permits/mandates; reconcile journals/residual commands/physical state |
| Related changes to envelope/tool/fixture/policy, etc. | Block affected scopes | New qualification/epoch and new start intent |

Automatically clearing a simple hold requires **evidence that no intervention, protective reaction, authority/device generation change, or unconfirmed external change occurred**. If that continuity is unknown, take the revocation path. Reappearance of PASS alone does not establish lost continuity. Applications requiring automatic safeguard reset/restart are not declared automatically supported by this revision; record separate validation/extension requirements.

Block kinds are fixed to TRANSIENT and LATCHED. Only normal WAIT_TARGET, admission-observation holds without safety reactions, and **simple lost RPC responses while querying the original invocation's result** may be TRANSIENT. The last case still requires physical-state/authority continuity and absence of safety reactions/human intervention. Other revocation/intervention/protective-reaction events are LATCHED. After fresh evidence and continuity checks, explicitly record ClearTransientBlock without creating a new mandate. LATCHED does not clear automatically when condition values recover.

P is the persistent owner of TRANSIENT blocks. Do not replicate/latch their block IDs on H. H checks its own current local conditions each time; those current results change when sources recover. A local protective reaction/latched anomaly detected by H is a separate LATCHED block and is not erased by P clearing a transient block. FenceCell/ArmCell block IDs target only LATCHED blocks.

## 4. Operation DispatchPermit

A RunMandate is not a native-execution token. P issues a `DispatchPermit` for each operation by combining existing T1 with cell conditions. It binds **operation ID/intent digest, purpose, cell/envelope/qualification revision, cell_epoch and scope vector, grant/fence, Host boot, expiry, and evaluation evidence**. It cannot be used for another action, Host, or run.

Permit states are `ISSUED / CONSUMED / VOIDED / EXPIRED`. Duplicate transmission of the same permit retrieves the prior receipt without adding native execution. At most one permit is valid for an operation. Do not issue a new permit without durable Host confirmation that dispatch has not occurred. Existing SEND_ENTERED operations permit only lookup/cancel/recovery.

TTL is mandatory in envelope/profile. Do not claim stopping performance from a common fixed value. P/H verify the same-host-boot/CLOCK_BOOTTIME conditions of common v1.0; permit `expires_at` is fixed on that timeline. Do not restore old permits after Host restart. TTL renewal is unavailable; expiry requires a new evaluation and new ID.

## 5. Final Host gate

In **the same command gate**, the Host checks the following and serializes permit consumption, SEND_ENTERED persistence, and entry into native submission.

1. Peer/contract capability and matching target/Host/cell membership in CellDefinition.
2. Matching cell_epoch and all dependency scope epochs, Host boot, grant/fence, purpose/operation/digest/permit ID, and expiry.
3. Permission under the Host-maintained block set and related open intervention/change scopes.
4. Native mode, source freshness, MAINTAINED/ADMISSION conditions, and actual device interlock conditions for this operation.
5. No conflict with existing operation/cancel/native results, and permit not consumed.

H stores permit consumption and SEND_ENTERED in the existing delivery journal in **one local transaction**. Persistence failure permits no normal native effect. An entered native call does not become “a call that never happened” through cancellation/epoch change. Do not hand resources to a new owner or use them as access evidence while delayed/stalled native calls remain. High-rate samples follow separate session/single-consumer rules but may belong only to sessions opened with this cell's permit/epoch.

## 6. Invalidation propagation and guarantee scope

When P detects latched invalidation, atomically record the block, new epoch vector, related mandate revocation, permit invalidation, events, and outbox. Then propagate `FenceCell` to affected Hosts. In its gate, H fences unentered commands from previous scope epochs and durably acknowledges `FENCE_INSTALLED`. If scope definitions are ambiguous, expand to the whole-cell boundary.

P does not activate the new epoch for operation until every required Host acknowledges. After H installs the new fence, old permits must not allow new native entry. Already-SEND_ENTERED calls, native queues, movement, and support require separate reconciliation; fence acknowledgement does not mean they have ended or that access is allowed.

H's cell_epoch and per-scope epochs are durable maxima and cannot decrease. Do not overwrite the current map with an old full vector. Reject smaller cell_epoch requests; at the same cell_epoch, any scope value below the stored value is STALE. Replay at an equal epoch returns the existing receipt only for the same payload/request ID. Process FenceCell and ArmCell through the same gate; ArmCell targets only the exact current vector and explicitly named block-ID set. It cannot erase later blocks/new events. A new cell definition specifies retired-scope tombstones and new membership without making old permits valid again.

**Limits of distributed observation:** delay exists between P discovering a loss and H installing a fence. An already-issued permit may arrive at H in that interval. RX does not guarantee that all physical actions disappear immediately at P's DB update. Validated local detection/protection paths must provide immediate reactions required by the hazard. Without such paths/timing evidence, do not qualify that envelope.

New condition values alone do not clear H's latched blocks. Require explicit new P evaluation/restart and epoch synchronization. Clear TRANSIENT according to §3. Invalidate changes outside RX, such as pendant/OEM actions, under profile observations/site change procedures. Record the possible existence of undetectable silent changes as a support limitation.

## 7. Atomic boundaries for recovery and change

| Transaction | Records committed together |
|---|---|
| Start | StartIntent key, RunMandate, current cell/envelope/qualification, run/executor binding, events |
| Operation admission | Existing T1 key/intent/activation/resource/outbox + cell-condition evaluation + permit/current epoch |
| Hold/revocation | Impact blocks, epochs and mandate/permit states, open intervention/change cases, fence outbox, events |
| Revalidation | Used evidence and case revisions, material/condition invalidation/new evaluation, dispositions/records |
| Restart | Expected case/cell/run revision and clearance checks, new mandate, new activation/explicit continuation, events |

Use the same transactions in the existing SQLite state DB. Do not assemble success across separate cell/operation DBs. H delivery/evidence journals remain separate factual records; do not claim the two DBs and physical effects form one distributed transaction. All mutations follow the existing rule of processing key duplicates before CAS.

## 8. Purposes and protective exceptions

Permit purposes are `PRODUCTION / SETUP / RECOVERY`. SETUP is not automatically a low-impact action; it must satisfy the envelope's conditions/roles. RECOVERY binds to a specific case/procedure/step and applies **explicit alternative conditions** distinct from normal production conditions. Prohibit `ignore_safety=true`, arbitrary guard omission, or automatic permission merely because speed is described as low.

Reads/record lookups do not require motion permits but enforce access rights. Preallocated local protective reactions operate to the necessary extent even without storage/network. Do not expand that exception into production, automatic restart, or arbitrary mode/torque changes. Calling a device driver's destructor during normal shutdown while support must be maintained must also satisfy this contract's lifecycle conditions.
