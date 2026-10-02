# Host execution-configuration binding v2

Status: contract, P-side client and durable P dispatch are connected. P integration
tests cover signed review through unqualified application with registered Host
identities and simulated v2 observations. Native Host implementation, qualification
activation and M3 acceptance remain pending.

The independent `rx.host.configuration.v2.HostExecutionConfigurationService` has
Inspect, Apply and Lookup methods. It uses the existing authenticated call/cell
envelopes and a distinct binding hash, without a fallback to the v1 service.
Payloads are at most 1,000,000 bytes and carry exact schema/hash/size references.
Requests, receipts and observations use explicit `rx.host-execution-configuration-*.v2`
schemas. Old bare v1 receipts/observations cannot establish v2 acceptance.

The v2 request embeds unchanged v1 scope/fence/context facts, plus an exact policy
for every target with a v2 recipe. Each policy pins its publication, artifact and
full value-selection policy, and the package/catalog/template sources for exactly
the templates owned by the addressed Host. Required baseline Intent digests must
match these templates; their presence does not authorize alternative parameters.
The initial v2 targets are SIMULATION only. Embedded context facts are not emitted
separately to a v1 Host as a substitute for policy acceptance.
An affected observation-only Host can have an empty local template/package set;
its required Intent set must also be empty, so this grants no operation capability.

An applied receipt correlates the complete v2 request digest and unchanged context
receipt facts. It additionally records the accepted publication/policy reference,
configuration, original request and receipt sequence for every v2 target. A
NOT_APPLIED receipt must have no accepted policies. Snapshot policy observations
must match their applied context identities. Currentness requires both the original
Host boot/journal/epoch/context checks and the exact requested v2 policy acceptance.
`activation_authorized` remains false; configuration acceptance grants no execution
permission or claim of native completion.

Before Apply, P must durably save the original complete request and send-entered
state. Any transport error, missing receipt or unsupported service is an unknown
application outcome; Lookup uses the original request ID and must not replay Apply
as a fresh operation. P's existing durable coordinator now chooses the exact
protocol for Inspect/Apply/Lookup. Typed variants retain flat v1 serialization and
cannot implicitly coerce a v2 request into a v1 transport. V2 task reads, receipt
recording, summary and apply proofs check the v2 policy evidence and digest, in
addition to unchanged scope/generation/fence/permission checks. Conflicting
receipts are retained as disputed; they never replace the original receipt.

V2 task records use a separate internal schema with immutable chunked blobs, at
most 8 MiB including duplicated request/receipt/observation context. This handles
valid wire records approaching 1 MiB without making acknowledgement persistence
fail on the old single-document limit. Each store document remains below 1 MiB.
V1 tasks stay in their original schema/encoding. The new task's original request,
digest and send-entered state survive reopening; applicability still requires a
fresh current-process read.

The temporary blanket v2 apply refusal is replaced by these normal durable
barriers. A P transaction test uses one current store/trust policy for signed process
and execution-template packages, then publication, independent process review,
impact approval, fences, v2 Host receipt and apply. It rejects v1 and mismatched
policy observations, blocks new apply after definition revision drift, and recovers
the original apply after a lost response. The simulated Host observations do not
prove native Host execution or frozen-binary case 2. Legacy Host qualification/recovery adapters
explicitly refuse v2 receipts until their policy-aware paths are connected.
