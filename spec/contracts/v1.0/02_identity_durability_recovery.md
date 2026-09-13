# Identity, Durability, Authority, and Recovery

Normative: RX Contract v1.0 · [Semantic contract](01_responsibility_and_semantics.md)

## 1. Identification and identity

| ID | Issuer/scope | Reuse rules |
|---|---|---|
| installation_id | Installation registration, UUID | Independent of device names/IPs. Identifies the original history owner even after DB restoration |
| store_generation | Restore/initialization procedure, UUID | Retained on normal process restart. Changed and reconciled after backup restoration/history loss |
| run_id / activation_id / operation_id | platform, UUIDv7 | Persistent IDs for run, step visit, and operation respectively. Timestamps do not establish causal order |
| request_key | UI/business client, UUID; persistent activation+slot for BT | scope=(installation, client namespace, API method, key). Preserve client namespace across connection/authentication-token renewal |
| invocation_id | UUID issued by Host at PREPARED commit | Issued once per operation. Preserves linkage to native IDs such as ROS goal UUIDs |
| boot_id | UUID at every process start | Distinguishes process generations. Must not be confused with an actual device boot ID |
| journal_id + seq | Each persistent journal | seq increases monotonically from 1. Continues across process restarts; journal replacement changes the ID |
| resource_fence | Durable increasing integer issued by platform per resource | Reject values below the largest value received and stored by Host. v1 runs only one Runtime |

The same request_key and canonical intent return the stored operation. A different intent produces `KEY_CONFLICT`. Identical content with a new key may represent new production work, so do not merge by hash alone. DB unique constraints/transactions ensure concurrent requests create only one operation.

Submit key matching checks the `(run_id, activation_id, slot)` binding tuple as well as intent_digest. Reusing a key for another step produces KEY_CONFLICT even if the content matches. For general mutations, the key fingerprint is the canonical method and business body excluding session/call/key/expected_revision and server-issued IDs. Credential/connection changes and CAS retries do not change business intent. Cancel must include the original operation/digest/cancel_rule; Recovery must include the original operation/procedure/evidence/disposition. A changed audit reason also constitutes a distinct request.

The Host's **delivery journal** (PREPARED/SEND_ENTERED, etc.) and **evidence outbox journal** have separate journal_id/seq spaces. A receipt at delivery seq=20 and the first evidence at outbox seq=1 are valid. P deduplicates receipts by (delivery journal ID, seq) and evidence batches by (evidence outbox ID, seq). Receipt lookup values must also be recorded as immutable evidence in P's T2 before being used in decisions.

v1 **retains a small tombstone for the lifetime of the installation history**, containing operation ID, scope/key, intent digest, latest outcome/integrity, and record location. Large observation/diagnostic data may have separate retention. An installation whose ledger is fully discarded is commissioned as a new installation and must not accept old keys as new operations. Stop new admission if retention becomes impossible. Do not revert an old key to “a request never seen before.”

## 2. Platform commit boundaries

The default storage binding is a transaction port over a single local SQLite DB. Durable commit requires WAL + synchronous FULL, a supported local filesystem, and validated storage hardware. Two containers must not directly open the same DB file. Backups that merely copy the DB file are prohibited; use a consistent backup procedure. SQLite is implemented in a storage adapter outside the Rust domain core.

| Transaction | Records included in the same commit | Allowed after commit |
|---|---|---|
| T1 admission | key→operation, canonical intent/digest, activation/slot binding, resource reservation, OperationAdmitted event, dispatch outbox | Host Prepare request |
| T2 evidence | Host inbox dedup cursor, immutable evidence, operation revision/decision, events, corresponding outbox ack | Durable ack to Host; expose UI events |
| T3 workflow | Existing checkpoint revision check, child allocation per activation/slot, new checkpoint, events/outbox | Return checkpoint acceptance to executor |
| T4 cancel/authority | Cancel intent or fence/authority change, events, pending Host delivery | Native cancel/authority reconciliation request |
| T5 recovery disposition | Investigation evidence/actor/reference to prior outcome, resource-release conditions, new-admission conditions, events | Permit new run/activation |

The outbox durably distinguishes NEW from EMIT_ENTERED. Before a network call, the sender performs CAS from NEW→EMIT_ENTERED; cancellation before emission performs CAS on the same row from NEW→VOIDED. A sender cannot emit a VOIDED row. After EMIT_ENTERED, a message may be delayed even if the Host has not seen it, so NOT_FOUND alone does not establish NOT_EXECUTED.

An outbox delivery ack is not physical completion. If the commit return is lost and commit status is unknown, look up the same key. Do not retry with a new key before confirming by lookup. If the local DB itself is corrupt and cannot be queried, retain STORE_FAULT.

For an operation belonging to an executor/run, T1 checks run=EXECUTING, the current executor authority generation, activation execution eligibility, permitted slot/visit, and recipe/schema/profile revisions in the same transaction. Pause/Abandon/step completion and new-slot admission are serialized on the same run revision/CAS boundary. Lookup/result retrieval for an existing key remains allowed after the run stops, but a late unused slot must not become a new operation. Pause does not mean that an EMIT_ENTERED native operation has been canceled; perform separate cancel/recovery. Standalone operations without a run are accepted only in setup/manual roles and modes permitted by site configuration, with the same profile/resource/current-condition checks. Separate API permissions prevent the UI from converting production requests to standalone operations to bypass run checks.

The executor authority generation is the current executor Session ID bound to RunView. StartRun binds the run to the current authenticated session of the peer designated in site configuration. T1's CallContext.session_id must match it. If E restart/disconnection creates a new session, reject late requests from the old connection and leave the run PAUSED/RECOVERY_REQUIRED. After retrieving checkpoints and reconciling unresolved operations, an explicit StartRun binds the new session. Creating a new session alone does not automatically resume an existing run.

## 3. Host journal and single native delivery

The Host stores its inbox/delivery/evidence outbox on a solutions-specific persistent volume. This is **a record of native delivery facts the platform cannot directly know**, not a second authoritative ledger of business state.

1. On `PrepareOperation`, check identity/profile/generation/resources/authority. Durably store `operation_id, digest, invocation_id, state=PREPARED`.
2. On `AuthorizeDispatch`, recheck the Host's locally valid grant, startup generation, freshness, interlocks, and device session immediately before dispatch.
3. Durably commit the same invocation as `SEND_ENTERED`. Process-wide serialization and journal CAS ensure exactly one actor performs this transition.
4. Only the actor that successfully commits may call the native request, at most once. Including SDK-internal retries, v1 profiles prohibit automatic repetition of native effects. The physical meaning of read-query retries versus byte-level transmission retries must be distinguished and validated.
5. Durably record native responses/results in the evidence outbox and publish them to the platform. Do not delete them before the platform's durable ack.

**Race between checking and calling**: the Host command gate serializes grant/fence checks, SEND_ENTERED CAS, and entry into native submission with cancellation, grant revocation, and handover. An implementation that only checks inside the gate, releases the lock, and calls native code later is prohibited. If an entered native call does not return or its thread stalls, do not hand that resource to a new owner. Local protective reactions use a separate path, but do not by themselves establish that a pending call has disappeared.

When the Host cancels PREPARED, it durably records a **VOIDED_BEFORE_SEND** tombstone through the same gate/CAS. Delayed Prepare/Authorize requests look up this tombstone and are rejected. Cancel requests include the digest so that explicit cancellation of an operation not yet in the inbox can also create VOIDED_BEFORE_SEND with operation_id+digest. Only one of tombstone and SEND_ENTERED can win first. If SEND_ENTERED wins, cancellation cannot declare non-execution; reconcile native result/stopping instead.

Recovery from PREPARED establishes that the native call has not occurred. A crash at SEND_ENTERED may have happened before the actual call or after device execution. **Leaving this interval UNKNOWN is the chosen guarantee.** Do not automatically call again. Do not claim atomicity between database records and physical actions.

Duplicate `AuthorizeDispatch` calls return the current receipt. Continuing a PREPARED operation after reconnection requires a new control grant and current-state checks. At SEND_ENTERED or later, only lookup/reconciliation is allowed. Even if native code supports deduplication, v1 retains the prohibition on retransmission after uncertainty. Only a future explicit extension for a specific binding may relax it.

Cancellation binds a separate `cancel_id` issued by T4 to the target operation/digest/cancel_rule. The Host maintains a PREPARED→SEND_ENTERED→result delivery record per cancel_id and does not repeat the native cancel call on duplicate cancellation delivery. Original production receipts and cancel receipts are separate. Requesting another stop with a new cancel_id requires fresh checks of current authority and the profile's repeated-stop conditions. Loss of a cancel response does not justify automatically creating a new cancel_id. Control samples use the session/seq rules of §6 without per-sample fsync; local protective reactions use the storage-independent exception of §8.

## 4. Authority and resources

Distinguish user permission, workflow execution authority, and device command authority. UI login alone does not grant robot command authority.

- The Runtime uses a single writer and an exclusive OS process lock to prevent two active instances. Do not replicate the same storage across multiple computers and automatically activate them.
- The Host owns actual command endpoints. It journals the maximum fence and valid grant for each resource and rejects older-fence requests.
- Before approving a new owner, invalidate the previous owner's pending requests and confirm local stream closure, residual native commands, and material support. Without an ack, resources remain QUARANTINED. Lease expiry alone does not transfer them to a new owner.
- Old commands already in OS/network/device queues are also part of handover. Revoking the RX gate does not establish that native queues are empty. Profile evidence must confirm closure of old native channels/sessions, completion of pending calls, and exclusion of residual device commands. If this cannot be checked, automatic handover is prohibited.
- Other clients or pendants directly calling native APIs may not check RX tokens. Profiles must specify native mode/remote-authority detection or OEM exclusive-integration mechanisms. Without them, explicitly limit the claim to RX-only exclusivity and narrow the permitted scope.
- Conflict units are the profile's `resource_set`, such as controller/bus/mode/physical workspace, not logical `arm/gripper` names. Sending an arm and a gripper through the same JTC uses the same command resource.

v1 reserves every resource needed at operation start in one transaction and prohibits waiting while holding a partial allocation. If resources are unavailable, return BUSY with a retry hint. Acquiring additional resource subsets later is prohibited until deadlock-avoidance rules are added. Only nonconflicting parallel operations are permitted.

A new fence for a bundled resource_set is calculated as `max(stored fence of each resource)+1` and stored for all resources in one transaction. The Host accepts an equal fence only for replay of an existing request with the same owner/grant. A new owner/grant cannot reuse an equal fence. If a new fence is recorded before actual resource handover completes, the resources remain quarantined.

## 5. Time and grants

Separate time limits by purpose: RPC deadline limits response waiting; admission validity limits an intent not yet dispatched; execution timeout triggers observation/recovery; observation age determines evidence validity; lease limits command authority; deadman limits stream-input validity.

When issuing a grant, the Host retains `grant_id, host_boot_id, resource_fence, receiver_expiry`. Expiry is evaluated only against the Host monotonic clock. Do not directly compare a peer's wall-clock deadline. The requester submits an opaque grant and sequence number. Renewals accept only increasing renew_seq values from a live authenticated session; duplicate renewals do not extend expiry. Renewal after expiry is rejected and requires a new authority-acquisition procedure.

Independently fix the operation's `prepare_validity_ms` against Host monotonic time at PREPARED so indefinite grant renewal cannot cause a long-waiting action to execute later. After Host restart, do not restore that validity; leave PREPARED awaiting reconsideration. Wall-clock changes or clock-synchronization failures must not extend command lifetime.

Device execution timeout is profile-defined, not an arbitrary global 5-second value. If the previous monotonic timeline cannot be restored after restart, record elapsed execution time as unknown and reconcile without new production actions. Recorded UTC supports event display; it does not determine causality between different actors.

For v1 deployment on the same Linux host, P/H must verify and share **the same host boot identity and CLOCK_BOOTTIME timeline**. Different time-namespace offsets are prohibited. Identify this as `clock_id=<host-boot-uuid>/boottime`, distinct from application process boot_id. A connection unable to verify this condition cannot validate current-state freshness to the required scope, so relevant operations are not admitted. Future deployment across hosts/operating systems requires a separate transport profile with validated clock conversion and uncertainty bounds.

The upper age bound when P evaluates current conditions is `P.now - H.receive_time + acquisition_uncertainty`. It requires identical clock_id, GOOD quality, a freshness_basis permitted by the profile, and finite acquisition_uncertainty. An observation that waited in H's outbox therefore does not become fresh on arrival at P. If source time can be mapped onto the same timeline, also check the stricter source-age bound. Unknown original cache time/seq requires CACHED_UNKNOWN, which cannot serve as valid evidence of current conditions. Immutable native completion history for a past invocation differs from a current predicate and is not discarded merely because it is old.

## 6. Control streams

Opening a CONTROL_SESSION requires fixing its source, command schema, mode, resource, deadman, and expiry reaction first. The process producing follower/leader/policy output connects to the Host inside solutions. Do not relay real-time control samples through platform RPC.

Each sample carries `session_id, source_boot_id, source_seq, host_ticket, values`. A short-lived ticket issued by the Host binds Host monotonic expiry. Reject samples that arrive late with an expired ticket. For multiple samples on one ticket, accept only increasing seq; a ticket does not extend its original expiry. The sample cache contains one latest valid value per resource; replay of prior values and disk queues are prohibited.

A sample's session_id is **the CONTROL_SESSION operation_id itself**, distinct from the authentication Session ID. H binds it to grant/resource/source_boot/schema/mode. The last accepted source_seq increases monotonically across the entire (control operation_id, source_boot_id) pair and is not reset on ticket replacement. Even if two tickets overlap, do not reapply samples with seq less than or equal to the already-received seq. A source_boot change requires closing the existing session and creating a new operation.

Actual native sample application is also serialized per resource through a **single consumer/command gate**. Acceptance handlers must not each execute native writes independently. The consumer selects the latest cache entry, checks session/grant/ticket validity and the last applied seq within the gate, and proceeds through native submission. Another sample application, session closure, or authority handover cannot intervene between checking and submission. Applied seq increases monotonically; if an entered call stalls/becomes uncertain, apply the §3 rule prohibiting transfer to a new owner. Prohibit inversion where a delayed seq10 handler resumes after seq11 has been applied.

Tickets alone do not solve sources repackaging stale observations as new samples. Source observation-age checks and freshness provenance are also mandatory stream-profile conditions. Source/Host restart, mode change, grant expiry, or deadman activation closes the session and blocks new samples. Execute the local expiry reaction and then observe its result.

Record closure reasons such as REQUESTED, EXPIRED, SOURCE_LOST, MODE_CHANGED, DEVICE_RESET, and STORE_FAULT. Distinguish processing a closure request from confirming actual stopping/support. Sending `0 velocity` is itself a model-specific reaction command; do not impose generic torque-off on humanoids.

## 7. Restart and reconnection

| Event | Discard/invalidate | Allowed after recovery/checks |
|---|---|---|
| Platform-only restart | Unemitted authority from the previous Runtime session; stale UI readiness | DB lookup, Host receipt/evidence retrieval, fence reconciliation. Preserve recorded operation IDs. Do not resume runs automatically |
| Host-only restart | All volatile grants/control sessions/samples and local monotonic validity | Distinguish PREPARED from SEND_ENTERED in the journal; check current device state/mode/residual commands |
| ROS controller/PLC/device restart | Validity of current observations/mode/native-result cache/control session | Recheck native boot evidence and profile. Preserve completion history, but re-establish current-state predicates |
| No device restart detection mechanism | Boot continuity cannot be guaranteed | Set a new device-session boundary on suspicion/connection renewal and recheck required states. Do not claim detection of silent reboots |
| BT executor restart | In-memory tick state and local pending promises | Retrieve platform checkpoint and activation-slot mapping. Use existing operation outcomes instead of reissuing with new IDs |
| DB backup restoration/journal loss | Automatic execution, previous leases, old outbox delivery | Change store_generation, compare last watermarks with Host, quarantine all related resources, and reconcile on site |

There is no guarantee that the application alone detects administrator cloning/volume rollback that conceals the rollback itself. Supported operation requires a restore procedure that changes store_generation and cross-checks with the Host. If both sides lose history simultaneously, prior operation outcomes are UNRESOLVED and new commissioning is required.

## 8. Storage failures and graceful shutdown

If platform DB writes fail, block new production admission/dispatch. If the Host can retain evidence, keep it in the outbox. Host journal failure also blocks new native effects. Do not record an already-performed physical action as canceled because persistence failed.

Predetermined **local protective reactions** must function during storage failure/network disconnection. Profiles and device control define the specific stop/hold/transition. This path is the explicit I12 exception that does not wait for journal commit. Preserve available evidence and report unknown outcomes/record gaps after reconnection. A software protective reaction is not itself a certified safety function.

The graceful shutdown sequence is `block new operations → request stream/operation cleanup → confirm residual commands and material support → confirm torque may be released → stop driver → flush journal`. If a driver's destructor disables torque, do not gracefully stop that driver while torque must remain enabled. Expiry of a graceful-shutdown timeout does not automatically authorize forced termination. Physical consequences of forced kill/power loss require separate device design and validation.
