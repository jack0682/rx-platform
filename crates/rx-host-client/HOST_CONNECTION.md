# Host bootstrap, lease, and current-state inspection

2026-09-11. Added `ConnectedHost` and `ConnectionService`. They connect to a specified Host, compare contract/identity/ledger/source generations, and perform Fence → grant → P registration. This is distinct from qualification, Arm, and production start.

## Read contract

Uses the separate optional `rx.host.read.v1` Inspect. Frozen base/cell manifests are unchanged. Base/cell negotiation and a registered client certificate are prerequisites; no request key/CAS is accepted. The response is a canonical `rx.host-snapshot.v1` artifact bound to SHA-256/size/schema.

The Host snapshot contains Host ID/boot, delivery/evidence journal IDs, cell definition/envelope/environment, epochs/scopes/block IDs, resource fence maxima, pending operation/permit IDs, and requested native observations. Per-source generation, schema/unit, acquisition time, uncertainty, quality, and evidence ID are preserved.

NativeAdapter's `observe_sources(cell, ids)` is a read-only port unsupported by the default implementation. If unsupported, it returns sources_available=false and empty observations. Device generation is not replaced with Host boot, and connection success is not converted into READY. FileDevice provides only an explicitly simulated ready source. Physical ROS/PLC source mapping remains a subsequent adapter obligation.

## Verifying the actual peer

A HostClient pinned connection compares the actual leaf DER SHA-256 in addition to configured CA/client key and server-name validation. The custom connector completes the TLS stream before passing it to Tonic. Only Tonic's internal transport URI is HTTP, preventing duplicate TLS; the request origin remains HTTPS. Input endpoints allow HTTPS only, and no path returns a plaintext connection. Success and rejection of incorrect pins were verified while the actual test Host required mTLS.

The Host's authenticated evidence producer must first be registered with P, and the cell contract must be negotiated. Host boot/evidence journal in the read snapshot must match that producer. Another journal or incarnation produces attention. A P user/Host session is not created by trusting only the server's claimed Host ID.

## Persist before delivery

P PrepareHostLink checks current access, software lifecycle, cell revision, definition/envelope, source descriptors/generations, and remote scope. It does not proceed if the Host remembers a higher epoch than P or has unknown blocks. Pending work prevents creating a new lease.

A grant fence is durably allocated above both the Host's persisted resource maximum and P's already allocated maximum. Several cells must not independently replace valid grants for the same Host resource. New acquisition is rejected if it conflicts with another existing registration's incarnation/source or if an active run exists. Complete coordination of one shared grant across multiple cells is not yet provided.

The plan stores two request IDs, a platform-side Host session, Host incarnation/ledger/source generations, target scope, resource fences, TTL, and preparation time. The same current plan is reused. If delivery/evidence journal IDs change within the same Host boot, reuse/reacquisition is rejected even in a new transport session, preserving the original plan/request IDs. Preparation/commit are bound to a 100 ms current-snapshot window.

Over the network, the planned Fence and AcquireGrant are sent, and responses are checked against the same Host boot/journals/target. P expiry for the grant is **the durable lower bound at planning time + TTL**. Resending the same grant request does not extend expiry using a later transmission time.

CommitHostLink rechecks P's latest cell/producer/allocated fences and the responses, then atomically stores HostRegistration, Fence evidence, and the original commit receipt. Repeating the same request returns the original result; a different response body is a conflict. Changes to P state before commit cause rejection. An unused grant does not create native work.

## Renewal and service lifecycle

Lease renewal also first persists the request ID/sequence and original send lower bound. A lost response reuses the same pending renewal. If Host boot, grant ID, owner/resources/fence/TTL, or current cell scope has changed, existing authority is not newly adopted. Late responses do not cause validity to be recalculated from retry time.

ConnectionService waits for a registered publisher and attempts initial connection with backoff up to 2 seconds. After connection, it attaches the existing Dispatcher and renews before lease expiry. Failure publishes Attention to internal state subscriptions while retaining the sender for fencing/evidence processing. It does not automatically reapprove a new Host boot or source generation or resubmit native operations. When the owner closes, it exits after completing the sender's current bounded pass.

## Platform execution configuration

Optional `host_links` in `rx-platformd` startup configuration:

| Field | Meaning |
|---|---|
| host, cell | Host/cell currently registered and permitted in P |
| uri, server_name, server_fingerprint | TLS endpoint, DNS/IP name to validate, exact leaf digest |
| tls.certificate/key/ca | Existing pinned-file format. P client certificate/key and Host server CA |
| ttl_ms | Lease of 1000–30000 ms |

Duplicate host/cell entries and out-of-range TTLs are rejected. Key files use existing private-file checks. P's API/diagnostics start even without a Host link. The presence of this configuration neither creates a driver/Host process nor grants qualification. The S runtime draft's default entrypoint also still does not start devices.

## Boundaries that remain separate

- Continuous, atomic snapshot observation ingest and independent maintained-condition expiry monitoring are connected. See [observation ingestion](../rx-application/OBSERVATION_INGESTION.md). Connection registration itself does not make P conditions PASS; actual observations are assessed separately against current source/configuration and age criteria.
- Restoring an incarnation changed by P/Host reboot to operational registration requires an explicit recovery/rebind procedure. Re-promotion of execution authority is currently rejected, preserving the original operations/lease. Observed source generation changes from the same authenticated Host are recorded as raw evidence and withdrawal; the new generation is not approved for operation.
- Host connection/inspection/delivery state subscriptions connect to the [diagnostic registry and operator view](../rx-runtime/HOST_SERVICE_HEALTH.md). A complete service summary in the local platform status file remains future work.
- Actual ROS/native source readers, automatic Host process/driver activation, shared-resource multi-cell lease coordination, and full supervision/qualification remain incomplete.

## Validation scope

Core tests cover preparation/commit rollback, response loss, request conflicts, boot/journal/source/pending/epoch rejection, and renewal sequences/timing. Actual TLS integration with a separate Host process checks pinned connection and incorrect-pin rejection, bootstrap reads, automatic initial registration, and retrieval/renewal of the same lease alongside existing native submission, response-loss, and two-material-attempt paths.

Incoming producer registration in this integration is an explicit fixture at the validated credential adapter boundary. Actual Host → P publisher negotiation is validated by existing separate TLS tests. Do not expand this into a claim that one product launcher creates both directions through complete automatic configuration or physical testing.
