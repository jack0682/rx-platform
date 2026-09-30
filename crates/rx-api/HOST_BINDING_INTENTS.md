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

The HTTP surface does not accept uploaded observations as confirmation. The registered
Host configuration worker collects bounded reads and submits them; a failed read records
its reason without discarding an earlier confirmed commit.

Preparation of a binding change fences only the baselined or confirmed-committed Host
generation. Host configuration dispatch and application are allowed only while every plan
Host is COMMIT_CURRENT: its confirmed commit was read from the Host generation registered
now, under the Host's active producer session, and no later read contradicted it. A Host
restart after confirmation demotes it until an explicit re-admission
(`/api/v1/hosts/readmission`) and a new confirmation. Application still leaves the change
APPLIED_UNQUALIFIED; qualification is separate. 
After a P restart the requests of the previous runtime report `HOST_BINDING_BASELINE_REQUIRED`.
`POST /api/v1/process-change/host-binding-intents/adopt` (same Transition wrapper,
registered-terminal ReleaseManager) carries them into the current runtime without changing
their request IDs, baselines or confirmed commit, provided the change is still staged and
current and the clock of the original request is the current clock. The previous record is
kept as history and the confirmation session is cleared, so the Host must be read again in
this runtime before it counts.

Current limit: a P restart re-registers the package intake service with a new generation,
which makes every device review, and with it the process review of the change, no longer
current. Adoption is therefore refused (`QUALIFICATION_REQUIRED`) until that review
currency question is resolved.
