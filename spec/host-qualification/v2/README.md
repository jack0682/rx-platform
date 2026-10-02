# Execution v2 Host qualification acknowledgement

This is an implementation contract within the authorized execution-v2 revision,
using the existing v1.1 revision procedure. It does not change v1 qualification,
introduce a new review policy, authorize native effects, or pass the P/Executor gate.

## Purpose and prerequisites

A fixed-Intent v1 qualification receipt cannot approve a v2 derived input domain.
After reviewed application, P must link the independently checked qualification
report to the exact v2 configuration acceptance already stored by each Host.
The approved publication/policy and the original configuration request/receipt
identities must survive issuance, acknowledgement, activation and recovery.

The existing six-area report, independent reviewer, current package store/policy,
impact/fence checks, scoped terminal authority, quiet resources and explicit global
activation remain required. Only `rx.requalification-policy.v3`, with successful
complete derived-domain verification and SIMULATION targets, can issue execution-v2
qualification. A v1/v2 policy or a failed/NOT_RUN report grants no v2 domain rights.
A purpose is still required for every target; a purpose name is not an operation permit.

All stages that repeat derived verification (report record, decision, issuance and
activation) use the explicit v3 computation ticket upper bound of 600 seconds;
legacy tickets retain 30 seconds. Exactly 600 seconds is expired. Longer computation
time does not extend a session, fence, permit, Host-read freshness, registration or
revision lifetime. Commit must recheck all of them. Activation still requires fresh
Host observations within the existing three-second bound at preparation and commit;
CPU verification alone cannot refresh that observation.

## Separate transport and closed payloads

The namespace is `rx.host.qualification.v2`, service
`HostExecutionQualificationService`, with Inspect, Accept and Lookup methods.
Base CallContext and CellCall field numbers/envelopes stay unchanged. The service
uses a separate binding hash; there is no fallback to v1. Canonical request and
observation payloads are at most 1,000,000 bytes, with exact schema/size/SHA256
references. The closed schemas are:

- `rx.host-execution-qualification-request.v2`
- `rx.host-execution-qualification-receipt.v2`
- `rx.host-execution-qualification-observation.v2`

Request embeds the unchanged v1 qualification request as context facts, plus
`policies`, keyed by cell. Each binding pins the publication Reference, execution
policy ArtifactRef, full v2 configuration request digest and full v2 configuration
receipt digest. At least one v2 binding is required. Mixed affected cohorts may
include v1 cells, but **every v2 cell in that cohort must occur exactly once**.
P derives this set from its applied configuration records. Host independently
compares it with its durable v2 configuration receipt; omission is not opt-out.

Before Accept, Host must validate the full configuration receipt and correlate
Host identity/incarnation/journal, change, configuration digest, context request
and receipt sequence. The policy/publication and both configuration digests must
match exactly. The qualification dependencies must include the policy, report
index, input closure and local signed package/catalog/template assets represented
by that configuration. The template baseline Intent set must match the local
policy templates. No caller-provided equality or opaque policy hash substitutes
for these stored facts. An observation-only Host has no local operation templates
or Intent rights; it still acknowledges the same policy identity for its cohort.

The outer request digest covers both the legacy context digest and v2 bindings.
Embedded v1 context is never sent separately to an old Host as an equivalent approval.

## Receipts and currentness

Receipt embeds the exact v2 request/digest and the unchanged v1 receipt facts.
ACCEPTED additionally records an exact per-cell policy acceptance: publication,
policy, configuration receipt digest, qualification ID/revision, qualification
request ID and acceptance sequence. NOT_ACCEPTED has no accepted v2 policies.
Receipt validation checks the complete outer request and all links, not only the
embedded legacy digest. Configuration and qualification receipt digest domains
are distinct; a receipt from another request/cell/Host/revision cannot be reused.

Observation retains the unchanged v1 configuration Snapshot and AcceptedCell facts,
plus current configured v2 policy stamps and accepted v2 qualification stamps.
Each stamp must match the corresponding snapshot/applied/accepted identities.
`receipt_matches_current_host` is true only when **both** legacy currentness and
all v2 policy/configuration/qualification identities match. Lost/replaced policy
state cannot remain current merely because legacy epochs or qualification IDs match.
Bare v1 observations and receipts cannot acknowledge a v2 task. A boolean asserted
by a caller must equal the result of these checks. `activation_authorized` remains
false: Host acknowledgement alone never issues native dispatch permission.

## Durable P state and activation

P saves the complete original request before entering SendEntered. Unknown transport
outcomes query the original request ID; they do not allocate fresh acceptance work.
Any explicitly permitted missing-receipt retry retains the original request/digest
and existing continuity checks. Conflicting receipts preserve the original and
mark the task disputed. A policy mismatch, generation loss or disputed active
acknowledgement suspends the affected qualification using existing barriers.

V2 durable Host-task payloads may use the existing bounded immutable chunk store
(up to 8 MiB including duplicated context); each database document remains below
1 MiB and reader barrier10 is required. V1 records/bytes and their limits stay intact.
The complete affected Host cohort must acknowledge v2 policy identity before global
activation. P checks current definitions and the full package registration again
at issuance, activation and new-effect readiness. Definition revision drift blocks
new effects even when numeric values match. Immutable receipt lookup/reconciliation
for original operations remains available; it never renews authority.

Activation certifies the reviewed bounded domain, not arbitrary parameters. Each
new execution still needs the authoritative Run/actual-object/slot selection,
deterministic recomputation, selected parameter/index membership, normal permission,
fences and operation permit. Cross-Run operation-selection approval reuse is rejected. This contract
does not add cold Host restart/R7 recovery or change UNKNOWN settlement rules.

## Required prechecks and implementation order

Positive controls must link signed packages and the real P review/issuance/activation
transactions. Negative controls include v1 downgrade, omitted or wrong policy,
foreign configuration request/receipt, stale or missing configured/accepted stamps,
changed definitions/registration after CPU verification, conflicting receipt,
missing-ack activation, 600-second boundary expiry and legacy 30-second retention.
Tests use protocol fixtures before new Host implementation, and are labelled as
such. Frozen installed v1 binaries still provide counterexample2; current v1 source
branches are not substitutes. New Host and UI remain after contract/P/Executor
cases1–5, followed by the single compatible release and separate human M3 acceptance.
