# Host recovery communication HTTP boundary

`host_recovery.rs` is a BFF connecting to `rx-runtime::host_recovery::Service`. Success on this path means RecoveryOnly communication, not a grant, Arm, operation admission, or native replay authorization. The product Host client worker uses the pinned transport registry from startup configuration and submits actual reads to the writer. HTTP input does not accept snapshots, URIs, certificates, paths, RPC methods, or principal/role/session values.

## Inputs and responses

| Method/path | Strict input | Response |
|---|---|---|
| GET `/api/v1/host-recovery-context` | Query `host`, `origin` | `{context, context_digest, expected_cells}` |
| GET `/api/v1/host-recoveries` | Query `host`, optional `after`, required `limit`(1..50) | `{items:[{view,proposal_digest}],next}` |
| POST `/api/v1/host-recoveries` | `{request_key, command:{host,origin,expected_context,expected_cells}}` | `{view, proposal_digest}` |
| GET `/api/v1/host-recovery` | Query `id` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/approve` | `{request_key, command:{id,expected_revision,proposal_digest,expected_cells}}` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/progress` | `{id}` | `{view, proposal_digest}` |
| POST `/api/v1/host-recovery/query` | `{id,operation}` | Typed `QueryResult` |

`context_digest` and `expected_cells` come from the server's `Context` method; `proposal_digest` is calculated by stored `Binding::proposal_digest()`. The UI does not reimplement JCS or digest semantics. Mutation envelopes and all inputs reject unknown/duplicate fields; ID/Digest/Counter use existing domain JSON representations. The writer retains final responsibility for mutation CAS and idempotency ordering.

`Binding.requested_context_digest` is an immutable request correlation value preserving the original Prepare.expected_context. If blockers found during proposal preparation are added to actual `binding.context`, the diagnostic context digest may differ from the original request. The HTTP server returns Propose responses after comparing the original request digest, Host/origin, cell revision set, and current user's principal. Stored proposed_by.session/terminal are historical identifiers from proposal time; they need not match current values when the same principal logs in again or changes registered terminals and retrieves the original key. HTTP and core recheck current authentication and access to every cell; stored identifiers are not promoted into current authority. The UI reviews the same proposal using the opaque server-provided `proposal_digest`, without removing blockers or recalculating the context digest.

The list lets another ReleaseManager or a new browser discover stored recovery records without depending on locally stored UUIDs. It checks current actor authentication first, then the 1..50 size limit, before selecting the service. Core access filtering and the original next cursor are preserved. Listing creates no ownership, approval, or Fence.

Propose/approve forward the original request_key unchanged. After response loss, retrieve the same content with the same key. Progress continues only the existing Fence task/request ID of an already approved binding; it creates neither approval nor a new Host key. Query retrieves the result/original receipt for the selected existing operation. It must not become an operation re-execution path.

`QueryResult.lookup` is `NOT_NEEDED`, `PREFIX_OBSERVED`, `UNAVAILABLE`, or `UNSUPPORTED`. Evidence is diagnostic material correlated with the selected operation, with `evidence_complete:false` and `operation_authorized:false`. The original Host publisher must deliver the exact complete prefix. HTTP rejects a View with operation_authorized=true or a QueryResult with operation_authorized/evidence_complete=true as an incorrectly connected service response.

## Authentication and availability

Existing direct terminal mTLS, Host, Origin, cookie/terminal binding, CSRF, and JSON Content-Type checks are retained. Each handler reads the current writer's UserProfile to check the ReleaseManager role and registered terminal before exposing worker availability. Worker/core recheck current session, role, terminal, installation, and the complete Host-cell scope during actual I/O and authoritative transactions. HTTP preflight itself is not reusable authority. The loopback development router has no configured worker and synthesizes no registered terminal.

Unauthenticated requests receive 401; forbidden roles/terminals receive 403. A valid ReleaseManager receives 503 `HOST_RECOVERY_NOT_CONFIGURED` when no worker is configured. `WriterError` preserves existing API mapping and does not hide outcome_unknown=true on lost writer responses. Worker transport errors are limited to 503 `HOST_RECOVERY_UNAVAILABLE` (outcome_unknown=true), unverifiable reads to 409 `HOST_RECOVERY_INVALID_READ`, and worker saturation to 429 `HOST_RECOVERY_BUSY`. Original transport error text, certificates, paths, and internal RPC content are not returned. After network loss, returning an approved binding in FENCING state is a normal recovery-progress response, not completed activation.

`TerminalHttps::new_with_host_recovery` accepts a package worker, optional S operator bundle, and optional recovery service together. Existing `new`, `new_with_package_intake`, and `new_with_operator_ui` constructors delegate with recovery=None, preserving existing callers.

## Validation scope

`crates/rx-api/tests/host_recovery.rs` is test source using actual terminal TLS/SQLite writer to verify authentication, historical response retrieval after relogin/registered-terminal changes, current role revocation, unconfigured-service responses, strict input, CSRF, forwarding of original IDs/keys/CAS, writer-error preservation, and server digest output. RoutingMock supplies only delivery-error and output-DTO test material; it does not synthesize Host recovery success/authority. Actual pinned Host worker, Fence, recovery binding, and lookup validation are exercised in product worker/core tests and separate acceptance. These HTTP tests alone do not establish acceptance of recovery or operation.
