# Optional resident reporting binding, revision 3

This optional service records attributed Supervisor registry snapshots. It does
not open a frozen Host/Executor/Operator session, grant execution permission,
establish physical ownership or replace frozen base/cell mutations.

Open requires a registered mTLS certificate mapped to an active principal whose
only role is Observer, exact installation/store generation/shared clock/release,
and this binding's hash. A separate peer boot identifies the reporting process.
Reconnect with the same peer boot and certificate binding preserves its session;
a new boot or P incarnation retires that session for current reporting.

An Engineer component owner or AccountAdmin grants a reporting scope to that live
session. It pins P component/revision, source registration/revision and catalog.
Only that peer may inspect/publish within it. Revocation or a stale peer refuses
new reports. Browser credentials never authenticate this service.

Publish carries canonical JSON for `rx-domain::resident_reporting::Report`, its
SHA-256, a canonical UUID request key and binding hash. Payloads are limited to
65536 bytes. Unknown/duplicate fields and noncanonical payload bytes are refused.
Reports are ordered per diagnostic lineage/instance, starting at 1; same-key recovery returns
the original receipt and conflicting reuse is rejected. The source binding is
immutable for an instance. Declaration change or retirement does not erase the
permission to record diagnostic history under an active scope; the owner read
marks that registration revision noncurrent. A report is not a new assignment.
A terminal Exited/NotStarted report cannot return to Running.

Responses are canonical JSON plus schema and SHA-256: `rx.resident-reporter.v1`,
`rx.resident-reporting-scope.v1`, or `rx.resident-report-receipt.v1`. The receipt
time is P's packet acceptance time, not process observation time. Its fixed basis
is REPORTED_REGISTRY_SNAPSHOT, execution ownership is NOT_ESTABLISHED_BY_REPORT,
and work use is NOT_EVALUATED. Registry history is not fresh OS evidence.

Base/cell protobufs and manifests are unchanged. Existing clients that do not know
this optional service cannot use it; this is not a claim that frozen services
support general component management. Legacy registration migration, trusted
content intake, P-directed launch and operating admission remain separate work.

The component owner/AccountAdmin manages scopes through the ordinary BFF:
POST `/api/v1/components/reporting` takes an idempotent request key and a command
containing component, expected_component_revision, reporter_session,
source_registration and source_revision. GET `/api/v1/component/reporting-scope`
with `id` reads the current scope/revision; POST `/api/v1/components/reporting/revoke`
takes scope and expected_revision. Only the existing authoring rights apply.
These do not make an Observer an Engineer. Cached mutation replies are historical
results; the reporter service rechecks actual current permission on each call.

GET `/api/v1/component/report` with component and instance returns an owner-only
view. `reporter_session_current` describes the accepted peer incarnation and
scope, not heartbeat or process liveness. `registration_revision_current` compares
the declaration cut; it is not readiness. Retiring or changing the declaration
preserves diagnostic reports under an active scope and marks the old cut
noncurrent. Only explicit scope revocation stops that metadata publication.

S retains each original registration/run/instance identity in the report's source
binding. An explicit P scope relates it to the P component; this is a diagnostic
relation, not silent import, alias replacement or migration of the old writer.
The Rust client publishes exactly once per call. Keep its request key and report
on response loss; retries are explicit. New reporting-process incarnations need
new scopes through the explicit continuation procedure below.

Revision 2 adds owner-approved diagnostic continuation and scoped head reads.
POST `/api/v1/components/reporting/continue` takes scope, expected_revision and
reporter_session with the ordinary idempotent mutation envelope. It atomically
revokes the predecessor and records a new scope with previous_scope/root_scope.
Only an active predecessor can be continued, so it cannot branch after a commit.
The original component/revision, source registration/revision and catalog remain
fixed, even if the declaration was later changed or retired. This approval is
for metadata continuity only; it cannot adopt a PID, restart a process or grant work.

Head takes the current session, scope and instance and returns canonical
`rx.resident-report-head.v1`. Its optional receipt is the latest stored report in
that scope's diagnostic lineage. Unrelated scopes cannot read or append to it.
An empty head means no record in the current store, not proof that an earlier
store never accepted a request. Continuations retain sequence order across peers;
old receipts remain immutable, and the old scope/session cannot publish again.
No report can resurrect a terminal execution through continuation.

The delivery outbox commits each request key, exact report and original peer
before sending. It retries that request only within the same live reporting
session/scope. After explicit owner continuation, an identical latest receipt
can recover the prior acknowledgement. Otherwise the original request is retained
as PRIOR_DELIVERY_UNRESOLVED; a new attributed snapshot does not rewrite its
outcome. A recorded receipt regression requires explicit store reconciliation.
Latest source snapshots may coalesce before a request is created. This service
is not a lossless journal of every OS state transition.

Compatibility: revision 1 binding hashes are refused by revision 2 peers.
Old stored scopes decode with no continuation and become their own lineage root;
no persisted source ID or old receipt is rewritten. Revision 1 binaries cannot
read new continuation rows; transparent downgrade is not supported. Frozen base/cell contracts
are unchanged. Reauthorization is an explicit owner action after P/reporter
incarnation changes; a transport reconnect within the same process reuses its
peer boot. New process starts must generate a new boot.


Revision 3 adds ReadAcceptance for the historical target receipt of an imported
component. The request carries current reporter session, owner-issued reporting
scope, source freeze ID and binding hash. P requires an active canonical scope
(component ID equals source registration ID), the matching import origin and an
Accepted transfer. A diagnostic alias or a report about an unimported component
cannot authorize this query. No complete source history, configured path or
credentials are exposed. The returned atomic transfer cut includes the original
freeze record, target receipt digest and acceptance time, and identifies the
current authenticated peer, scope and canonical component. This is metadata
provenance within the owner's reporting scope, never operational authority.

S checks the exact local source cut, target installation, current peer/scope and
component before recording it under the still-sealed source. The original source
freeze and registration namespaces remain immutable. Repeated reconciliation of
the same target decision retains the first local evidence; a changed target
receipt requires investigation and cannot overwrite it. The stored record is
historical: it cannot extend a session, report scope, process lifetime or work
permission. Reconciliation failure preserves the previous evidence and fences.

Compatibility: revision 3 peers refuse revision 1/2 binding hashes. P and S must
upgrade together and establish current reporting sessions/scopes. Existing
reported observations, original IDs and scopes are not rewritten. Base/cell
contracts are unchanged. This addition does not provide P execution assignment,
package content acceptance or process ownership transfer.
