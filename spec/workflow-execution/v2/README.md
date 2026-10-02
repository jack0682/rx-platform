# Explicit workflow execution v2 input contract

Implementation in progress; no public v2 execution ingress is enabled yet.
The authoritative revision decision is [Docs #96](https://github.com/jack0682/rx_docs/pull/96)
and the [semantic contract](https://github.com/jack0682/rx_docs/blob/cc1c0fd839c8aa6060a598ff8ff3e47ea18c56ad/docs/contracts/workflow-execution/v2/README.md).
v2 is the execution version; v1.1 names the existing revision procedure.

`execution_v2` adds closed data types for policy, exhaustive report index and
run-bound selection. `Policy.templates` pins node -> host/normalized finite
Program Intent; `node_contracts` pins the same node keys to implementation,
version, primitive and each parameter's type/unit/frame. Only the parameter
ArtifactRef can vary in a selected action. The fixed template timeout remains
unchanged; the resolved timeout must be positive whole milliseconds within it.

The input closure contains the exact workflow reference, label/spec, complete
definition records and ordered candidate requests. Its reference uses raw SHA-256
of canonical bytes. All candidate requests have slot zero in the closure; the
selected slot is set only during deterministic resolution. Context digests use
`RX-EXECUTION-CONTEXT-v2` with slot normalized to zero. The actual runtime object
instance and its checked values/revision belong in the separate selection record.

The materializer verifies source definition digests, workflow content and object
model binding, calls the existing bounded resolver, and rejects invalid/bounded
reports. It checks the package parameter contracts and emits canonical
`rx.workflow-parameters.v2` assets and `rx.execution-report.v2`. The report binds
the input closure, template digest, compiler identity, candidate/model/slot, full
resolution/provenance/constraint result and concrete node actions. The report
commits to each parameter's bytes through its ArtifactRef. Parameters refer to
inputs/templates, not their containing report, so hashes are acyclic.

Canonical JSON is the existing JCS serializer: quantities use finite `Real`,
counters remain decimal strings, index tuples use bounded JSON integer indices.
Artifact readers check their byte limits before deserialization, require exact
canonical bytes, reject duplicate/unknown members and verify hash/schema/size.
Policy is at most 128 KiB, input closure 16 MiB, report index 2 MiB, report
900,000 bytes and each parameter 64 KiB. The legacy wire decoder stays at 1 MiB.
Index decoding validates the entire ordered Cartesian domain once and creates a
private immutable `ValidatedIndex`; candidate lookup then uses its array position.

Content digests of artifacts/reports are raw SHA-256. Structured policy, context
and template digests use the existing `domain + newline + JCS bytes` convention,
with domains `RX-EXECUTION-POLICY-v2`, `RX-EXECUTION-CONTEXT-v2` and
`RX-EXECUTION-TEMPLATES-v2`. Compiler identity uses
`RX-WORKFLOW-MATERIALIZER-v2` over the materializer, v2 DTO, canonicalizer and
primitive-type source bytes. Qualification must additionally pin the actual
runtime/dependency release closure; this source identity alone is not qualification.

P's `prepare_execution_inputs` reads all candidate inputs in one authorized
transaction and blocks stale workflow/definition references, even for unchanged
values. It returns a non-deserializable computation snapshot. It creates no
publication, Run, operation, receipt or permit. Final publication and every new
effect admission still need currentness rechecks in their own transaction.

Selection comparison assumes the expected record was read from P's authenticated
durable authority boundary. It compares the full record, including Run/Part,
object identity/revision, slot, publication/configuration, generation and report;
it checks exact parameter bytes and the invariant Intent fields. A caller-built
selection DTO or successful pure comparison cannot grant execution rights.

Remaining gate work: immutable publication and qualification dependency evidence,
durable Run/Part selection, explicit P/Executor v2 negotiation/admission and frozen
deployed-v1 injection. Existing v1 source/wire/binding manifests remain unchanged.
No Host/UI v2 implementation is authorized by these pure/preparation tests alone.
