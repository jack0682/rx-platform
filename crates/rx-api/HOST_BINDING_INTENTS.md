# Pre-recorded Host binding intents (unreleased)

`POST /api/v1/process-change/host-binding-intents` accepts the usual mutation wrapper
with a process-change Transition (`change`, `cell`, `expected`, `plan_digest`). A
registered-terminal ReleaseManager scoped to the full impact is required. The change
must be current and STAGED with a simulation Host binding plan.

P allocates one durable request per change/Host, with the original Host plan,
before/after configuration and cohort identities. Retrying the same mutation returns
the same records; changing content under its key is refused. The request is recorded
before any Host operation. Initial phase is AWAITING_BASELINE, and
`activation_authorized` is false. Issuance does not authorize Host commit, change
configuration, send fences, restore qualification or start work.

The internal observation handler accepts only the matching currently registered
Host session and full cohort. Its comparison rejects old/future reads, changed P
runtime, wrong Host boot, missing baseline identity, same-boot replacement, wrong
request/plan/configuration and either changed journal. A completed request cannot
be used to manufacture its own before-baseline. Metadata matching is not an
application/qualification result, and a stored last match is not current authority.

The current HTTP surface does not accept uploaded observations as confirmation.
The registered Host transport worker still needs to collect and deliver bounded
reads. Existing preparation/application barriers are intentionally unchanged until
baseline acquisition, fences, current confirmation and final revalidation are wired.
Adoption after P restart remains to be implemented without replacing the original
Host request. This endpoint is an integration primitive, not the finished deployment
workflow.
