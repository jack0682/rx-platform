# Host execution v2 transport

Normative operation boundary: rx_docs commit b4109940, workflow-execution/v2/operation-admission.md.
The separate service wraps existing authenticated Cell/Host request facts without changing v1.
Prepare carries canonical rx.execution-operation.v2 bytes, a content-SHA256 ArtifactRef,
and exact parameter bytes (64 KiB maximum). The binding includes the authoritative selection.
Authorize and both queries require the same RX-EXECUTION-OPERATION-v2 digest.
Replies echo that digest; a mismatched/missing binding is rejected before receipt/evidence ingestion.
The protocol hash is RX-HOST-EXECUTION-BINDING-v2 over canonical binding.json.
No v1 fallback, no inferred no-effect outcome on unknown/unsupported response.
P client/transport fixtures do not establish native Host execution conformance.

Revision 2026-10-03.2 requires Host-owned approval membership before native input handoff. The shared host_inputs verifier regenerates the selected report from the Host-retained input closure, compares its saved index entry, and compares exact parameter bytes. The gate must bind that material to its durable qualification acknowledgement; the pure verifier alone grants no authority. Python and external adapters use the same BoundInput meaning. See rx_docs workflow-execution/v2/host-input-membership.md and python-execution-profile.md.

Revision 2026-10-03.3 (rx_docs e609541f) defines common native entry/completion separation.
Host retains its gate through profile-confirmed entry and can then return NATIVE_ACCEPTED
while collecting completion through the existing journal/evidence path. Entry is not completion;
pending custody/UNKNOWN and all existing limits remain. Each profile defines admissible entry
evidence; Python's owned-runner acknowledgement is not a requirement on other providers.
