# Host recovery communication integration

This module retrieves records and outcomes for existing operations reviewed by an administrator after P restarts on the same store while the same Host remains running. It does not restore operational HostRegistration, grants, qualification, Arm, or authorization for new operations. The first physical cell is NOT_COMMISSIONED.

## Registration evidence and current checks

The first normal pinned connection atomically preserves the exact public TLS certificate/deployment/contract pins, actual Host process configuration, source snapshot, P configuration and producer, and final registration receipt. The [immutable baseline](../rx-application/HOST_BINDING_BASELINE.md) is historical evidence of that connection. Its original contents do not replace current connection authority. Historical evidence for earlier unpinned registrations is not synthesized afterward.

The product daemon creates a per-Host transport registry only from deployment configuration and registers declarations with the current Runtime. Actual network checks are separate. Neither the UI nor an HTTP body can provide URIs, certificates, or device snapshots.

The per-Host mutex in `recovery::Worker` serializes duplicate progress within the same worker. The application writer owns final authority and race checks. Private `RestrictedHost` exposes only connection, configuration/observation reads, Fence, existing receipt lookup, and existing native-result lookup. It exposes neither `Deref` nor a raw HostClient accessor.

## Administrator and execution sequence

1. The current ReleaseManager on a registered terminal inspects all Host cells, runtime origin, and existing registration/operation/unresolved restrictions.
2. Propose using the server-returned context digest and set of cell revisions. The worker collects actual pinned reads, and the writer rechecks the same scope.
3. Approve using the reviewed proposal digest and revisions. Recheck the current session, role, terminal, and permissions for every cell.
4. The writer first records send entry for the same Fence request ID/body, then the worker sends it. If exactly one existing runtime Fence exists, use its original outbox. Do not choose arbitrarily among ambiguous candidates.
5. Preserve the ACK on the original Fence. Late responses and responses inconsistent with current conditions remain facts but are not automatically promoted. RECOVERY_ONLY requires matching subsequent actual observations.
6. Query only existing operations within the approved scope through receipts and their original invocations. Even if the response contains a selected-operation prefix from the native evidence read, it does not claim to be a complete stream. The normal Host publisher delivers the complete ledger order, and P applies the original evidence admission rules.

If a proposal/approval response is lost, retrieve the same request key/body. Even when the historical proposal's session/terminal differs from current values, recheck current authority for the same principal. Progress continues the existing approval and task; it does not create a new approval. Network errors alone do not send a new grant or device action.

## Timing and diagnostics

The 100ms checks on configuration/observation read windows and current age remain in force. The custom TLS connector bypasses tonic's default connector, so it directly applies TCP_NODELAY. The same setting is applied to explicit incoming streams for P ingress and H RPC. This transport setting prevents small HTTP/2/TLS records from being delayed by Nagle/delayed ACK interactions; it does not extend validation deadlines.

Initial bootstrap failures record only Host, cell, stage, and a structured error code in `rx.host-connection-error.v1`. Prepare failures also include numerical configuration round-trip and source capture/age measurements. Remote error text, metadata, keys, and bodies are not logged. Repeated failures at the same stage/from the same cause are not printed again even when timing measurements change. The error state remains the existing Attention state.

## Tests and remaining scope

Distinguish core counterexamples, actual terminal TLS/API routing tests, P-only restart/API/browser tests using the two actual images, and known-operation lookup tests using actual P and a separate native fault fixture. Results and exact executed sources/images link to each phase's evidence in the documentation repository. The existence of test code is not a passing test record.

Host restart, source generation changes, new installation/store generations, measurements of a shared Host with multiple cells, operational rebind, qualification reissuance, and explicit Run resumption are outside the completion claim for this RecoveryOnly boundary. Successful outcome lookup also does not replace evidence of physical support handover or automatic resource release.

Native success is distinct from overall operation success. If the original completion rule has postconditions and restart broke permit/cell continuity, Work remains UNKNOWN/NONE even after retrieving the original capture and admitting the complete evidence prefix. A current true observation is not retroactively used as completion evidence for an earlier operation. The [actual acceptance tool](../../tools/HOST_RECOVERY_TEST.md) checks this case together with the absence of retransmission.
