# Host execution-configuration binding v2

Status: contract and P-side client implementation; durable change dispatch and new
Host implementation are still pending. This is not qualification or M3 acceptance.

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
as a fresh operation. The new client provides transport/integrity/correlation
checks only. Persistent dispatch and application gates must consume this evidence
before the temporary v2 Host-binding blocker can be removed.
