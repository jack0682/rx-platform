# Registration source intake

The optional `registration_sources` object in rx-platformd configuration enrolls
local legacy registry sources. It defaults to empty. Example fragment (merge into
an otherwise valid existing installation configuration):

```json
{
  "registration_sources": {
    "legacy-supervisor": {
      "path": "/var/lib/rx/legacy/registration.db",
      "owner": "engineer"
    }
  }
}
```

Paths must be absolute and normalized and remain under administrator control.
The owner must be an active Engineer or AccountAdmin when importing. Enrollment
does not open the source or stop P startup when the source is unavailable.
An authenticated owner/admin first reads
`GET /api/v1/registration-source?source=legacy-supervisor` to obtain the binding.
The legacy source must have been frozen for this P installation using the
Supervisor's explicit development freeze operation. Import it through the
existing authenticated browser API and CSRF policy:

```json
{
  "request_key": "<new UUID retained for exact retries>",
  "command": {
    "source": "legacy-supervisor",
    "freeze": "<original source freeze UUID>",
    "expected_binding": "<digest returned by the source context>"
  }
}
```

POST to `/api/v1/registration-transfers`. P reads and verifies the actual sealed
source, imports original UUIDs/revisions/history in chunks and atomically accepts
only the complete transfer. Existing UUID collisions are refused. Incomplete
components cannot be used through ordinary component APIs. Revoke existing
reporting scopes that use another P component as an alias for the source ID before
import; after staging, only the canonical ID may receive new report scopes.

Retain the request key and payload if a response is lost. Exact retry resumes a
partial transfer or recovers its committed receipt even when the source is now
offline. Current authorization still applies; a new P boot needs a new authenticated
session. Progress, receipt and paged original history are available at
`/api/v1/registration-transfer`, `/api/v1/registration-transfer/receipt` and
`/api/v1/registration-transfer/history`, each with `id=<freeze UUID>`; history
also accepts `after=<cursor>`. Failed validation leaves Receiving state for
investigation; there is no implicit abort, rebind or rollback/unseal operation.

Target intake explicitly sets reader schema 8; older readers refuse the target.
Ordinary stores stay at 6 and source-only freeze uses 7. Source history is archived
as data and cannot create P work/control authority. Target materialization times
are not claimed as original source timestamps. The source fence remains closed.

This does not transfer live process ownership, verify package content, grant work
permission, assign execution. The full contract is
[registration transfer revision 3](https://github.com/jack0682/rx_docs/blob/develop/docs/contracts/registration-transfer/v1/README.md).
Tests in transactions cover rollback, restart, current authorization, identity
collisions, history consistency and alias isolation. The opt-in runner
`tools/test_registration_intake.py --help` connects a real Supervisor source helper
to the actual P writer and HTTP route using copied, quiescent source evidence and
an independently preserved old reader. It never operates equipment.


A current owner-issued reporter scope for an imported canonical component can
read its historical target receipt through the optional reporting Acceptance RPC.
The response binds the original freeze/cut and immutable receipt digest to the
current peer/scope. Aliases, unrelated components, stale peers, revoked scopes and
wrong freeze IDs are refused. No paths or full source history are exposed on that
service. Supervisor's explicit reconciliation command checks and persists this
provenance under the source fence; it does not reopen local declaration authority.
Binding revision 3 requires a paired client/server upgrade and current scopes.
