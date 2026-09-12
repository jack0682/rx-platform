# Host qualification acceptance v1

Optional `rx.host.qualification.v1`. Exact binding hash, strict canonical payload reference/hash/size, existing authenticated base session and cell negotiation. All cohort cells must be negotiated for Accept. Inspect/Lookup are read-only; Accept is an idempotent Host metadata transaction.

A request binds a reviewed qualification identity to each current process context, complete Host cohort, current binding/boot/journal, epoch/scopes and exact blocked fence. It cannot expand static Host intent/purpose/envelope coverage. A receipt is ACCEPTED or NOT_ACCEPTED; transport failure is unknown. Same ID/different body or same review/revision with a different request ID conflicts.

Acceptance never Arms, clears blocks, issues a grant/permit, calls native submit/lookup or proves a physical test. It records the qualification identity eligible for a later explicit Arm and separately authorized operation. Native validation still requires current matching qualification, original allowed-intent coverage, narrowed accepted coverage, generation, grant, permit and local guard. Restart, process-context change or epoch/scope change makes old acceptance inapplicable. Cached receipt is historical; it never refreshes applicability or restores Arm.

Inspect/Lookup return original receipt plus current snapshot/accepted identities. The client verifies currency claims. Full metadata payload <=1,000,000 bytes and gRPC envelope <=1 MiB. P still owns human review, global multi-Host activation and operator intent. Host does not independently validate review document content.

A qualification ID/revision has immutable configuration/scope/evidence meaning, and revisions cannot regress. A replacement acceptance requires a newer cell epoch; exact same-ID receipt replay is the only same-epoch repetition. An empty accepted intent set authorizes no operation.
