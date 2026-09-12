# Optional production coordinator binding v1

This binding adds a consistent run/part read and authoritative part completion. It reuses frozen Cell.BeginPartAttempt for admission and does not change either base/cell manifest or the executor read/preparation bindings. Both methods require current registered executor identity, cell negotiation/assignment and this binding's canonical SHA-256.

Inspect has no mutation key or context revision. Its ReadPayload is canonical rx.production-state.v1 with exact content hash/size/schema. The read cut includes installation/store/runtime, caller session, cell revision/epoch/scopes, run checkpoint, ordered complete part list, immutable budget and current admission flag. It is available before the first part and after completion. A read is not permission to actuate.

CompletePart requires request_key, absent context expected_revision, and positive body run/part revisions. P checks current access before the whole key/body, validates cell/run/part identity, then CAS and active authority for a new completion. Only IN_PROGRESS may newly become CONFIRMED_COMPLETED. Graph completion must be rederived from actual P outcomes and release proofs. Caller BT SUCCESS is not evidence. An already completed part is returned without another revision/event/budget change. The original key/body always returns the original PartAttempt response. Changed body under the same key conflicts.

Part disposition, run checkpoint/revision, mandate exhaustion, events and response commit atomically. When every admitted part is confirmed and budget remaining is zero, P sets Run COMPLETED. Completion never refunds budget. No current material identity is invented: material_id remains absent until a real material identity is established.

Serial coordination may retire a planner context only after an authenticated P read proves that exact run/visit part is confirmed completed. Retiring such a context does not mean pausing the whole run. Stop intent, authority loss, unresolved parts, canceled/failed work and operator recovery remain distinct. Same-session sequential parts preserve the current run mandate; a new session/epoch is not automatically adopted.

This is an optional version-pinned coordination surface, not a generic disposition override. Setup and continuous-control workflows, material genealogy, large-run paging/capacity and complete recovery are not provided by this binding.
