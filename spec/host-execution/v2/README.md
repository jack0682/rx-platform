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
