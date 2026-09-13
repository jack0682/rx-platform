# Observation batch ingestion and maintained-condition expiry monitoring

2026-09-11. Connected the path storing Host observations as P condition evidence and the path checking validity when no new observations arrive. Observations do not substitute for qualification/grants/Arm/operation completion.

## Responsibilities and flow

| Location | Responsibility |
|---|---|
| S NativeAdapter / Host read | Return actual source generation/value/time/quality/evidence ID. Do not fill unsupported sources with READY |
| P HostClient / ObservationReader | Repeat authenticated Host reads and forward exact snapshots to the internal writer |
| P application | Compare current producer/Host/configuration, validate all sources, record atomically, evaluate maintained conditions and revoke authority |
| P independent expiry monitor | Reevaluate maintained conditions relying on old observations independently of network reads |
| P operating-authority handling | Continue to require separate qualification/start procedures/grants/permits |

Uses the existing optional binding of `rx.host.read.v1` unchanged. Frozen base/cell contracts and the shared SDK are unchanged. New `HostRead` and `BatchReceipt` are internal application types, not an API allowing external users to supply arbitrary approval states.

## Conditions for accepting a batch

1. The current Host role and authenticated session must be valid. The connection plan must be committed and match the current evidence producer, boot, both journals and session.
2. Current cell definition/envelope/environment and negotiated definition must match. A Host claiming epoch/scopes greater than P's is rejected.
3. Exactly the sources assigned to that Host must be included. Duplicate source/evidence IDs, omissions, schema/unit mismatches and out-of-range uncertainty are rejected before writing the entire batch.
4. Check clocks/order for read start, snapshot capture and P receipt; reject source acquisition times later than capture. The 100 ms initial grant-preparation window is not applied to observation recording. Delayed observations retain original acquisition times; values exceeding actual age limits evaluate as UNKNOWN. Historical values do not overwrite newer current records.
5. Source maximum age comes from P's FactSpec. Hosts cannot declare an arbitrarily longer validity period.

Accepting observations does not require an execution grant to remain valid or P/H epochs to match exactly. Observations of the same equipment/configuration are needed for investigation after stop/revocation too. A lagging H epoch can therefore be accepted, without restoring P epoch/blocks/qualification/mandates. Device generation changes are not adopted as normal observations.

## Atomic storage and evaluation

`report_facts` handles up to 128 sources for one cell in one transaction. It validates all descriptors first, stores immutable evidence and each source's current value, then evaluates maintained conditions. Intermediate states while updating sources one by one are not used for condition evaluation.

A test example is the maintained condition `A or B`. When A=true/B=false changes to A=false/B=true, the whole batch's condition remains true. Authority is not revoked based on the instant after only the first value changes. This guarantees consistency of P storage/evaluation, not physically simultaneous measurement of different sensors. Device profiles and adapters must separately establish required physical measurement consistency.

| entry disposition | Meaning |
|---|---|
| CURRENT | Stored as the current source record |
| DUPLICATE | The same immutable evidence is already the current value. Do not repeat the same state transition |
| HISTORICAL | Acquisition time precedes the current record. Preserve historical evidence and keep the current value |
| GENERATION_CHANGED | Differs from declared source generation. Preserve raw evidence, mark current quality unusable and revoke the impact scope |
| INTEGRITY_CONFLICT | Different content for the same evidence ID. Preserve original evidence/new contradiction and revoke the impact scope |

Integrity contradictions and generation changes are interpretable reports. Their events/blocks commit together with other valid observations in the batch and are distinguished in the result. In contrast, malformed descriptors or batches from another Host/configuration update no current values. Storage failures roll back the entire batch. Resending after a lost post-commit response does not change original evidence IDs. Re-ingestion dispositions describe current stored state, unlike fixed control-command receipts.

Repeated observation of a new source generation does not create another revocation for the same generation loss each time. Blocks do not proliferate while a disputed state persists either. Returning to disputed after a new normal observation records a new anomaly. Normal observations do not clear existing blocks or restore revoked mandates.

## Repeated reads and expiry monitoring

The connection service owns ObservationReader alongside dispatcher/lease renewal. Read interval is one third of the minimum maximum age among that Host's sources, bounded to 1–250 ms. Missed ticks are not replayed in a burst; each reader handles one request at a time. Read failures/unsupported responses become internal `Unavailable` state without fabricated false values or new timestamps. `Received` means a recording result was received, not source validity or a condition PASS. Periodic reads are not an event journal capturing every historical change or momentary signal; such evidence requires separate latched-state/event contracts from equipment.

A separate P monitor checks maintained conditions for currently active Runs/start attempts every 25 ms. It continues while a reader waits for a network response or fails. If maintained conditions are lost through expiry/uncertainty/failure, the same transaction revokes impacted mandates/permits and prepares Fence delivery. Original observation values/times/quality are not changed. An actually stale true remains a stale true.

25 ms is the current scheduling setting. It is not a validated worst-case response time including complete scans/writer waits/OS scheduling and does not constitute real-time protection. Actual profiles with short source TTLs require separate performance validation including acquisition, delivery and evaluation delays.

The P executable starts exactly one such monitor even without Host connection configuration. It surfaces writer errors as service failures rather than hiding them. Shutdown joins the reader and dispatcher; interrupted reads do not create native operations or restarts.

## Validation and remaining scope

- Test complete evaluation of two sources, rejection without writes for an invalid final source, failures before/after commit and retransmission.
- Test preservation of contradictions together with other source facts, and no block proliferation for the same generation loss/persistent disputed state.
- Test validity boundaries, expiry-monitor rollback/single revocation, and no restoration of previous mandates after receiving new values.
- Use `ConnectedHost.observe` and the repeated reader with an actual separate TLS Host. Automatic dispatcher/lost-response/two-part tests must continue to pass. The same tests also verify that repeated ingestion does not increase native execution count.

Physical source mapping/profile qualification, operator UI integration of observation/connection state, explicit rebind, field response times and record-retention policy remain incomplete. Current immutable evidence/event records grow with ingestion volume. Long-term capacity/retention/export policies are not yet provided, and continuous field-operation support is not claimed. The first physical cell is NOT_COMMISSIONED.
