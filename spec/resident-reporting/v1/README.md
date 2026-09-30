# Optional resident reporting binding, revision 1

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
Reports are ordered per scope/instance, starting at 1; same-key recovery returns
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
new scopes, and existing-instance origin changes require later reconciliation.
