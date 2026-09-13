# Automatic delivery and receipt reconciliation

`delivery::Dispatcher` is P's sender/reconciler, configured once per authenticated and negotiated Host. It changes the application only through the existing single writer. It neither directly invokes device SDKs nor decides outcomes from BT success values.

## Processing boundary

1. Read SQLite pending outbox pages with an exclusive key cursor. Rotate the cursor so earlier unresolved entries cannot permanently hide later ones.
2. `plan_delivery` checks current Host authority and message relationships. Initial delivery rechecks the current run/permit/conditions and changes NEW → EMIT_ENTERED in the same transaction.
3. Send Arm/Prepare/Authorize/Fence through the actual mTLS HostClient. Operation messages already at EMIT_ENTERED are queried through GetReceipt. Native invocations at SEND_ENTERED or beyond are not resent.
4. Record the receipt and completion of the relevant outbox entry in an application transaction. PREPARED acknowledges preparation delivery, not completed Authorize delivery.
5. Reconcile operations with known native entry through GetReceipt → Reconcile → the same T2. Outcome and resource handover are separate. This dispatcher does not automatically create resource release or part completion.

When another related delivery retrieves an identical receipt already stored, that delivery is completed too. Cached receipts do not bypass checks for invalid message/operation/Host relationships.

## Unknown states and retries

- Timeout/no response/NOT_FOUND is not evidence of NOT_EXECUTED. An EMIT_ENTERED operation retains UNKNOWN/quarantine and durable attention.
- If lookup for an earlier authorization returns PREPARED, retain REAUTHORIZATION_REQUIRED. Do not resend under the old grant/permit. Recovery with a new grant, current-state checks, and permit rebinding remains future work.
- Changed Host sessions/authority do not cause stale messages to be rewritten under new authority. Host reregistration belongs to the composition/recovery manager.
- If evidence arrives before the invocation receipt, preserve the source. Reevaluate it in the same transaction when the receipt establishes correlation. If native entry is already proven, consume the permit and discard any authorization still at NEW.
- Poll counts do not resolve unknown outcomes. Only the completion policy and actual evidence change outcomes.

## Operational bounds

Each pass processes 8 deliveries/8 reconciliations, prioritizing Fence within a page. HostClient RPCs are limited to 3 seconds. Other Hosts use separate loops. Retry backoff is 100ms → at most 5 seconds; scheduling metadata is limited to 1,024 entries. Unresolved outbox entries remain in the DB regardless of memory limits.

Shutdown follows completion of the current bounded pass. Already claimed sends are not canceled between CAS and RPC. These bounds do not guarantee site protection response time. Independent local paths provide physical protection; integration with product supervisor policies for admission stop, support checks, and shutdown remains future work.

## Current limitations

- Host links must already be authenticated and registered. This is not yet complete automatic discovery, new grants, rebind, or restart reconciliation.
- Reconcile returns the first 128-entry prefix of the Host journal. Complete delivery of a large journal requires a separate `publication::Publisher`.
- Reconciliation targeting and initial correlation reevaluation currently use entity scans. Large-scale indexing/measurement is incomplete.
- Attention is durable diagnostics and is not yet fully integrated with recovery cases/procedure UI.
- Product image/process configuration, public ledger wire mapping, visual editing/BT, native cancel/stream, and complete recovery/acceptance remain under implementation.

## Validation

The automated path in `tools/test_host_e2e.sh` uses an actual dedicated writer and a separate Host process. It drops the first Authorize response immediately after the simulated invocation completes. The dispatcher retrieves it through GetReceipt/Reconcile, verifying 2 Authorize calls and 2 independent device effects across two parts. It also verifies that a subsequent Hold Fence is applied to the Host automatically.

The test executor explicitly performs part/activation creation, handover proof acquisition/release, and part completion. Do not present this as a complete autonomous production service or physical robot acceptance.
