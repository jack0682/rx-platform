# Platform-owned resident declarations

An authenticated Engineer or AccountAdmin can register a component declaration
without installing a Cell or starting a process. The ordinary application writer
assigns the stable ID, records the creator, and atomically commits each version,
audit event and idempotent response. Only its owner or AccountAdmin, with current
authoring rights, may query or change it.

The declaration is the shared `rx-domain::component::Declaration`: a label and a
program/digest catalog reference. It is attributed input, not verified executable
content. Every view explicitly says content verification is NOT_ESTABLISHED,
execution ownership is NOT_ESTABLISHED_BY_REGISTRATION, and work use is
NOT_EVALUATED. Registration creates no Cell, Host, resource, permit or operation.

The public BFF uses the same writer and authentication/Origin policy:

| Route | Body or query |
|---|---|
| POST `/api/v1/components` | `request_key`, `command: {declaration}` |
| GET `/api/v1/component` | `id`, optional decimal-string `revision` |
| POST `/api/v1/components/update` | `request_key`, `command: {id, expected_revision, declaration}` |
| POST `/api/v1/components/retire` | `request_key`, `command: {id, expected_revision}` |

Recover a lost reply by sending its original key and body. A recovered mutation
returns its original declaration snapshot, even after a later edit or retirement;
use GET without `revision` for a current read. The authenticated principal is part
of the request fingerprint, so sharing a client namespace cannot alias another
author's result. Current rights are checked before replaying a cached response.

Updates use compare-and-swap and keep the previous snapshot. Retirement is
terminal for this registration and retains its history; it neither stops a
process nor clears any execution/resource obligation. Restart retains the ID and
versions while rejecting the old authenticated session. Registration is not a
mechanism for restoring operating authority.

Storage uses `component/` and `componenthistory/` in the existing Platform
repository. It adds no base/cell protobuf, normative manifest or public
control-journal mapping. The implementation remains excluded from the Host SDK.

This is the authoring portion of the resident ownership model. It does not yet
import legacy Supervisor registrations or bind execution reports, verify the
referenced package, supply a paginated discovery UI, or authorize execution.
Caller-selected IDs and authority fields are refused; legacy data must not be
silently adopted or independently rewritten in both P and S. The candidate
extension and remaining RF02 work are recorded in rx_docs under
`docs/contracts/resident-registration/v1/README.md` and the resident completion
ledger.

Tests cover SQLite pre-/post-commit failures, response replay, restart, current
role and owner checks, client-namespace aliasing, CAS, historical reads and
retirement, plus the real HTTP-to-writer path. These tests do not establish
physical support or completion of the resident framework.
