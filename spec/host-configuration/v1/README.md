# Host process-configuration binding v1

Optional `rx.host.configuration.v1` uses the frozen base Session and CellCall identities with strict raw protobuf decoding. Inspect/Lookup are authenticated reads; Apply requires a current Platform session, negotiated cohort cells and an exact request key. Every request supplies the binding hash. JSON payloads are strict, hash/size-bound and limited to1,000,000 bytes.

The accepted effect is **process context registration in the Host journal**, not driver/PLC/robot parameter application. Target definition/envelope/environment and required intent/condition coverage must match the currently approved Host bindings. A fresh full-Host quiescence observation and matching fence for every managed cell are required. Context/receipt/gate are committed atomically. Accepted context remains unqualified and prevents Arm until a separate qualification path is implemented.

APPLIED_UNQUALIFIED and NOT_APPLIED are durable facts. A transport/storage error is not either fact; caller must preserve an unknown outcome and query the same request. Historical receipts carry their original boot. The observation's context_matches_current_host compares metadata only; it is not a fresh physical-state or execution permission assertion. activation_authorized is always false.

P must persist request identity before sending and compare host boot/journal, change/preparation/plan, cohort target hashes and current fence context before using a receipt. Unsupported optional bindings cannot be silently treated as successful configuration acknowledgements. General native reconfiguration, live qualification/rebind, P mixed-configuration reconciliation and active-pointer selection are later integration work.


## Source revision 2 — binding commit observation

The optional `Snapshot.installation_identity` identifies the current running service
when its durable lifecycle marker is bound to the current Host boot. `Snapshot.binding_commit` records a completed local binding
transition only when the current service installation identity and semantic binding
digest match that commit. It includes original request/plan, target cell and before/
after configuration identities, installation identities, and retained journals.
Observation validation requires a matching current installation identity, the same
delivery journal/binding digest and a cell
inside the observed cohort. Historical receipt lookup is not this current observation.

P must acquire this through authenticated, time-bounded Host reads and correlate it
with the original deployment intent and current boot/cohort before using it. It is
neither process application nor qualification; activation_authorized remains false.
Missing evidence never confirms a replacement. P admission integration is separate.

The source revision/hash changes. Base protobuf fields and RPC names are unchanged.
Old JSON without the optional field still decodes; old strict readers cannot consume
the new field. Exact optional-binding hash negotiation requires matched P/Host/SDK
versions rather than silent fallback. The frozen base manifests are unchanged.
