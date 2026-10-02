# Explicit workflow execution v2 input contract

Implementation in progress. Saved Preview/publication HTTP paths are present;
no v2 operation or Executor ingress is enabled yet.
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

Remaining gate work: signed package verification and qualification dependency evidence,
durable Run/Part selection, explicit P/Executor v2 negotiation/admission and frozen
deployed-v1 injection. Existing v1 source/wire/binding manifests remain unchanged.
No Host/UI v2 implementation is authorized by these pure/preparation tests alone.

## Explicit signed template declaration

The declaration schema is `rx.execution-template-catalog.v2`, stored at
`execution-template-catalog.json` inside a signed `rx.package.v2`
`DEVICE_REFERENCE` package. A v1 fixed-operation catalog never implies permission
to substitute its parameter artifact. The declaration pins installation/cell,
SIMULATION environment, at most 16 named templates (host/normalized finite Program
Intent and exact NodeContract), and the family/profile/adapter source documents.
It has a 128 KiB bound. The initial profile is self-contained; locked package
dependencies and physical declarations are not accepted by this checker.

`VerifiedTemplates` consumes a package already reverified from the registered store
owner under the exact verification-policy fingerprint. It checks the new declaration,
source path/bytes/size, manifest asset declarations, present program/default-parameter
bytes, and exact installation/cell. A workflow template must match both its full
action and parameter contract. This is distinct from selecting a concrete runtime
parameter: only the qualified v2 publication/Run policy can approve that selection.

Qualification dependency extraction retains the manifest, signature, all declared
assets and **every signed file**, including files not used by a selected template.
The 1024-identity bound is checked here; the eventual combined publication closure
must also enforce the total bound across definitions, implementations and packages.
Canonical metadata aliases are retained; conflicting metadata is rejected.

This code establishes signed declaration integrity, not software review, source
semantic consistency, physical qualification or runtime admission. Fresh declaration
verification is mandatory in the publication HTTP path. Software-review and
qualification evidence still need connecting without weakening reviewer separation.
The preexisting v1 device-review
checker and its hashes are unchanged; a v1 approval cannot be relabelled as a v2
variable-input review. Do not close the P/Executor gate based on these checks alone.

## Saved Preview and publication

`POST /api/v1/workflow-executions/preview` accepts the existing request-key mutation
envelope with `PreviewInput`: id, candidate key/object-model/request tuples, slot
count, templates and node contracts. P takes a current authorized snapshot, computes
every candidate outside the writer, then rechecks access and the complete current
input cut in the commit transaction. Any invalid candidate aborts preparation.
No caller-supplied report/index is accepted as evidence. Completed Preview stores
the policy, input closure and exhaustive index; individual reports are regenerated
from the pinned closure and must match the saved index before being returned.

`GET /api/v1/workflow-executions/preview` reads the exact reference (catalog, id,
revision, digest query fields) and policy. `GET .../preview-report` adds candidate
and slot indices and returns the verified deterministic report. Computation stays
outside the writer transaction. Reads of historical Preview do not substitute
current definitions. A different installed materializer is refused, not used to
silently rewrite an old report. This implementation does not yet supply historical
materializer execution across release upgrades.

`POST /api/v1/workflow-executions/publish` takes id, exact Preview reference, cell,
and a complete node -> `{intake, template}` binding map in the mutation envelope.
Preparation checks current catalog/cell rights, saved inputs, package registration,
intake receipt identities and current configuration. No package I/O precedes those
checks. The existing bounded off-writer package worker then reopens each exact
stored object, reloads the pinned trust policy, verifies signatures/content and
matches each signed declaration to the saved Preview action and NodeContract.
There is no caller-supplied PASS field or fallback to a v1 fixed-operation catalog.

Commit requires the original boot, a ticket age below 30 seconds, current rights,
unchanged package registration/configuration and current definition closure. It
atomically saves an immutable publication referencing the Preview and policy,
including the cell, binding map, package manifest/signature/catalog identities,
complete package dependency refs and verification-policy registration.
The combined known definition/package/root count must fit the 1024 dependency
bound; later qualification must also account for runtime and implementation roots.
`GET .../publication`
reads the exact reference. Original-key replay returns the original receipt after
response loss; changed input under the key conflicts. Existing IDs cannot be
overwritten, and access is rechecked on replay. An old Preview remains readable
after a referenced revision changes, but a new publication is blocked. Stale
diagnostics include the pinned/current revision and digest and definition label.

These authoring publications are **not qualified or executable**. Template package
signatures are verified, but software-review and qualification linkage still need
integration before the P/Executor gate is complete. Neither publication nor a `NOT_QUALIFIED` projection
is a qualification approval. No Run/permit/device outbox is created by these APIs.

The existing qualification blob mechanism is shared at application level with
its original namespace/bytes/8 MiB bound preserved. Execution artifacts use a
separate namespace and a 16 MiB bound, with immutable 256 KiB chunks. Individual
store documents and legacy wire messages retain their 1 MiB limits. Policy/index
readers enforce their tighter limits and content/schema identities as well.

The first saved v2 Preview atomically promotes the local store reader barrier to
10; normal v1-only stores stay at 6 (or their already opted-in reader version).
Promotion rolls back with a failed transaction and never downgrades through an
older barrier call. Existing version-9-or-earlier binaries must refuse the store.
Sealed transfer of version-10 stores is unsupported and fails closed; existing
sealed-store semantics are not expanded by this revision. This source-level
barrier test does not replace frozen-binary counterexample 2.
