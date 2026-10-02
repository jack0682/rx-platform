# Execution v2 session, start and Part admission

This extends the implementation detail of the authorized execution-v2 contract,
not its scope. Existing session, cell, operator terminal, qualification, resource,
arm acknowledgement, mandate and budget rules remain in force.

## Executor negotiation

The registered Executor uses a separate `rx.executor.execution.v2` service and
binding hash. It must already own a current authenticated Executor peer session
and have negotiated the cell's current definition through the existing base/cell
contract. A browser/Operator session or a merely local Executor-role session cannot
make this declaration. The declaration records installation/store/runtime identity,
Executor session/principal, cell/definition and exact v2 binding hash. It is invalid
after session/peer/runtime replacement, definition mismatch or protocol mismatch.
A matching hash is a protocol declaration, not software attestation, a Run start,
a parameter approval or a native permit. There is no fallback to v1.

The v2 service carries bounded canonical request/reply artifacts with exact
schema, size and SHA256 references. Request keys retain existing mutation semantics;
read requests cannot carry mutation expectations. Unknown/mixed versions fail before
any new effect. Base authentication and cell-envelope field numbers remain unchanged.

## Explicit Start2

Start2 consumes the existing server-owned Prepared Run binding. Only Production
purpose and PartAttempt budget are supported in this profile, and the total budget
must equal the number of reserved entries. It cannot append slots or silently choose
another publication/configuration. Current instance binding is required for the next
Part, or the existing incomplete Part when resuming the established recovery path.
The Run-local ordinal and published slot ordinal remain distinct.

The operator's current terminal authority, exact revisions, qualification/purpose,
start conditions, current Executor session and registered prepared Host cohort are
checked by the existing start validator. Start2 additionally verifies v2 negotiation,
current publication/definitions/actual instance and reservation generation/ownership.
The existing arm handshake contains no substituted execution parameters and retains
its meaning; it is available only after exact v2 qualification acknowledgement.
At the final arm acknowledgement P rechecks all current v2 binding facts and Executor
negotiation before creating the ordinary mandate. A revision change during arming
cannot activate an old object/slot selection. Original acknowledgement/history remains
recoverable; it cannot create fresh authority after a failed new check.

Legacy Start/BeginPart paths cannot activate v2 records. Start2 does not bypass normal
Host checks or turn a qualification acknowledgement into an operation permit.

## Explicit Part2

The current negotiated Executor requests the next Part under its existing active
mandate and expected budget/cell state. P, not the caller, selects the next reserved
slot and previously recorded actual-object binding. Missing/stale binding, competing
ownership, wrong generation, ambiguity, incomplete previous Part, UNKNOWN/disputed
work, or exhausted budget blocks admission. No caller parameter bytes, report digest,
slot index or actual-object substitution can become authority.

P prepares an immutable computation ticket from the authorized saved domain/current
Run, object and reservation. Deterministic materialization occurs outside the writer.
The result must match the already approved candidate/slot index and include exact
report/parameter references. On commit, P rechecks actor/session/mandate, revisions,
qualification/current definitions, actual instance, reservation and budget at one
serialized boundary. A racing change either blocks commit or follows an admitted
immutable Part; it never rewrites that Part. Computation alone grants no authority.

The transaction persists the report and concrete parameter bytes, creates the normal
PartAttempt, consumes its budget once, links actual instance and slot ownership, and
records an idempotent result. Commit/reply loss recovers the same Part and artifacts,
not a new reservation or budget charge. Artifact reads are authenticated and limited
to the exact artifacts bound to that Run/Part. Unused approval from another Run is
not accepted even when content bytes happen to match.

Part admission still does not submit a node operation. Explicit operation-scoped v2
selection/admission, normal permission/permit/fence/resource checks and negotiated
Host validation remain required. Completion/UNKNOWN settlement uses existing evidence
and the approved same-slot retry/next-slot rules. No native Host or UI implementation
is claimed from protocol fixtures; frozen-v1 cases and the final single-bundle M3
acceptance remain mandatory.

## SubmitNode

`SubmitNode` requires an idempotency key and expected Cell/Run revisions. Its body
names only Cell, Run, Part, compiled node and mandate. P selects the published
workflow node, immutable Part report/parameter and reserved slot. A private 30-second
computation ticket re-materializes the approved report outside the writer; the commit
rechecks current instance/definitions/authority/frontier before the existing operation
and permit transaction. The bounded reply is `rx.execution-work.v2`, carrying Work and
its explicit immutable execution binding. Original-key/occupied-node recovery returns
the original operation without granting new authority. No caller Intent or parameter
reference is accepted. The per-Part graph substitutes only saved parameter references;
frontier completion still needs actual released operation evidence.

P sends such Work only through `rx.host.execution.v2`; receipt and evidence replies must
pin the original operation-binding digest. Full completion consumes the reserved slot;
abandonment, UNKNOWN and materialization alone cannot consume or replenish it. The
same-slot retry extension remains a separate acceptance requirement, not established by
these admission/transport tests.
