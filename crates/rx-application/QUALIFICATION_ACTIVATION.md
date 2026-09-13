# P qualification issuance, Host delivery and global activation

Status: phase54 implementation draft. Connects [requalification evidence and independent review](REQUALIFICATION_REVIEW.md) and [Host qualification acceptance](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/QUALIFICATION_ACCEPTANCE.md) to P's durable issuance/dispatch/activation. It does not replace actual site testing; test fixtures are scoped to SIMULATION.

## Three distinct events

1. **Issuance/PENDING**: Reverify current approval and the source package/policy, then pin per-cell qualification IDs and Host tasks in the ledger. Cell.qualification is not yet set.
2. **Host acceptance**: The Host binds qualification to the current complete cohort/context/generation and preserves a receipt. Blocks and inactive Arm state remain.
3. **Activation/ACTIVE**: P rechecks recent acceptance by every Host and current review/authority/originals, then atomically records qualification/commissioning/approved block clearance. Even here, no Run/Arm/permit/native command is created.

A subsequent separate StartRun by an operator checks current conditions, roles, Host lease and qualification. Arm completion, mandates, individual-operation permits and Host/native guards follow. Qualification activation does not mean operation execution.

## Policy v2 and existing records

`rx.requalification-policy.v2` requires a nonempty `purposes` set in each Profile. Only explicitly specified PRODUCTION/SETUP/RECOVERY values are permitted. P's general StartRun/active Run checks and the Host's acceptance-scope/permit checks enforce this. It does not mean all actual RECOVERY execution features are implemented.

Existing v1 can be read without purposes and retain requalification material/review records. An empty set is omitted during serialization to preserve historical canonical-byte semantics. Operating purposes are not inferred from v1 reviews, and the issuance worker rejects them. Adding new purposes requires a v2 policy and a new request/result/independent review.

## Issuance input and evidence

IssueRequest accepts review/cell, report revision/digest, decision revision, an expected_cells revision map for all impacted cells, and a per-cell list of clear_blocks IDs. The role is ReleaseManager on a currently registered terminal, and the intersection of user/terminal authority must cover the full scope.

Preflight and commit check the review's current policy/Runtime boot/application record/cell configuration/epoch/scope/block, latest approval/CAS and exact fence acks. Impacted Runs must be COMPLETED/ABANDONED, with no unresolved/disputed work, held/isolated resources or related open cases.

Outside the writer, the worker reverifies stored requalification originals/signatures and the current pinned v2 policy. It also reverifies the original package object referenced by the initial process review using the current package policy/Store. The actual StoredPackage owner/object/policy must match the private Prepared proof. Issuance/activation does not proceed if original content is unavailable or differs from current trust.

Issuance commit records Batch/new per-cell qualification IDs/revisions, restricted purposes/dependencies/evidence/limitations, per-Host task IDs/incarnations, sender identity, review slot and user request result in one transaction. A different issuance intent does not overwrite the same review/report/decision slot. The same key/body retrieves the original IDs. If a PENDING issuance's sender session has ended, a ReleaseManager on a currently registered terminal can reapprove the same issuance content with a new key. The worker reverifies originals/policy and updates only sender and Batch revision, without changing qualification/task/request IDs or bodies.

## Provenance of blocks to clear

For blocks created by new configuration preparation, P apply, requalification preparation and this qualification's suspension/runtime restart, an ownership ledger records `(block, cell, change, reason)`. Issuance input must satisfy that ownership and the actual current block. A reason name alone cannot delete a block.

Manual Holds, other cases/changes and old blocks without provenance records cannot be included for clearance. The clearance list may be empty or select only some verified blocks; the others remain. Thus, even activated qualification can leave Ready/StartRun rejected because of remaining blocks. No migration is provided that automatically infers and clears blocks of unknown provenance in previous draft data.

Final activation removes only the exact list in P. Actual remaining Host blocks are cleared during **Arm in a subsequent explicit StartRun**. The Arm outbox pins the set of clearable IDs approved by the qualification. The client reads current Host blocks and sends the currently remaining IDs only if every ID is in that set. It does not request already cleared IDs again. The Host does not treat Arm with partial clearance/remaining blocks as successful.

## Durable Host tasks and unknown results

Task state is AWAITING_SNAPSHOT → PREPARED → SEND_ENTERED, separate from receipt ACCEPTED/NOT_ACCEPTED. The worker compares the current Host snapshot with the original process-context request/receipt sequence from P application, then stores the exact Request/digest. It returns/executes the RPC only after the dispatch-entry commit completes.

- Storage failure before dispatch: Do not send to the Host.
- Lost response after dispatch entry/Host commit: Lookup the original ID first.
- No receipt: Do not infer NOT_ACCEPTED. Only this idempotent metadata request may be resent with the identical body/ID after rechecking the same Host generation/Binding/full context and current issuance/sender authority.
- Loss/contradiction after the first receipt: Preserve the original fact and latch disputed.
- Earlier unresolved/disputed dispatch: Do not bypass it with a new issuance. Fact lookup remains possible through the current identity for that Host.

Queries show per-Host accepted counts, partial acceptance, unknown results and current context. Global activation does not occur with only partial acceptance or unproven currency. Automatic replacement issuance/cancellation for NOT_ACCEPTED and detailed operational recovery UI are future work.

## Final activation

Activate input contains batch/cell, batch revision and a revision map for all cells. A ReleaseManager on a currently registered terminal requests it separately. The worker reverifies the source package/requalification evidence/policy, and commit rechecks authority, review/decision, quiet barrier and block ownership.

Every Host receipt must be ACCEPTED, undisputed and consistent with current boot/journal/session/Binding/process context/epoch. Query start time and observation digest are recorded as a P writer in-memory proof and used only within 3 seconds. This freshness is not restored after restart. A remote snapshot does not guarantee continuous physical stability; current session/guard checks are additionally required at native entry.

One transaction stores per-cell Qualification/COMMISSIONED/SETUP, approved P block removal, certificate→batch linkage, Batch ACTIVE/history/time, Change QUALIFIED_ACTIVE and request result. Cell/scope epochs retain the values acknowledged by the Hosts. Source package/policy/signature failures, CAS races and partial Host results do not partially activate any cell.

After a lost activation response, the same key retrieves the result from that time. That response alone does not establish that current authority remains live. Currency is checked through GET batch and the authority/condition checks for new actions.

## Maintenance, suspension and restart

New admission checks certificate configuration/epoch/scope, current registered policy/package service, the relevant review decision and Host acceptance context. Communication errors after activation are distinguished from current usability. Actual loss/contradiction of the same qualification receipt or loss of Host context suspends the cohort and records new epochs/fences and restrictions. Existing receipts are not deleted and results are not recreated.

Explicit Suspend, policy/package registration changes and Runtime restart preserve active qualification in history, remove it from the Cell, and record Batch SUSPENDED/Change APPLIED_UNQUALIFIED. The same suspended Batch is not reactivated. Restart requires fresh review preparation/generation/Host confirmation. This path does not arbitrarily clear separate restrictions such as user Holds.

The Host itself also checks every cohort member in the acceptance receipt and current accepted records/epochs/process contexts. If one member on the same Host changes, another member's old qualification cannot allow Arm/native entry. Actual device sessions are also rejected if they differ from their values at acceptance.

There is distributed delay between P ledger changes and actual fences on every Host. Immediate physical stopping/support is the responsibility of verified local protective paths; this implementation does not guarantee it through networks/DBs alone.

## Renewing existing leases after fencing

Only when an acknowledged fence matches the current P cell epoch/scope is that projection of HostRegistration updated. An old ack does not move the current generation backward. This ack does not renew grants/expiry/Arm.

Renewal must preserve the originally linked Host boot/journal/producer/source session, the same grant ID/fence/resource scope and definition. The current cell's required resources must be within the original grant. Under those conditions, the epoch acknowledging the new fence is allowed, without treating the initial LinkPlan/fence receipt as recreated. It does not reacquire an expired grant or expand its scope.

## API and implementation layout

| API | Meaning |
|---|---|
| POST `/api/v1/qualification-activations` | ReleaseManager on the current terminal: issuance preparation/original reverification/durable Batch |
| GET `/api/v1/qualification-activation?cell=...&id=...` | Engineer/Verifier/ReleaseManager: Batch/per-Host results and currency |
| POST `/api/v1/qualification-activation/activate` | ReleaseManager on the current terminal: global activation |
| POST `/api/v1/qualification-activation/suspend` | ReleaseManager on the current terminal: explicit restriction/suspension record |

Reuses the existing `{request_key, command}`, HTTPS authentication and CSRF. The public API does not accept arbitrary qualification IDs or Host Request bodies. The runtime qualification worker performs only current P tasks within the lifetime of the Host connection. Read proofs/network waits are not implemented as I/O waits in the single DB writer.

The application is divided into `qualification_activation` issuance/host_tasks/activation modules and shared checks. The runtime package worker handles original verification; the Host client worker handles dispatch/query/interruption. Executables configured with this path are marked as VERIFIED_REQUALIFICATION authority. The existing direct qualification port does not create qualification from startup values/Booleans.

## Verification and remaining scope

Follow the [phase54 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase54_checks.json). Ledger tests cover issuance/activation atomicity/lost responses, identical IDs, unresolved Lookup, current-context rechecks, policy revocation, receipt loss, partial Host acceptance, preservation of manual Holds, Runtime restart and rejection of purpose inference from v1. They also check renewal of the same lease after fencing and rejection of Host cohort violations/partial Arm.

A separate S simulated Host, the actual P writer/worker and registered-terminal HTTPS verify issuance, lost acceptance responses, activation and same-key retrieval. There are 0 native effects through activation. A subsequent separate operator StartRun connects actual Arm/prepare/authorize, native success evidence, handover observations, resource release and Run completion, verifying 1 effect in an independent file-device log. This does not represent actual equipment operation or physical acceptance.

Dedicated UI, long-duration/maximum-size/load validation, complete operational recovery for expired or unresolved tasks, physical qualification material/expert review/production trust deployment and updates, cancellation/rollback/upgrade restoration, and complete platform support remain incomplete. The first physical cell is NOT_COMMISSIONED.

Deployment scope: The S runtime image contains current executor/process tools and management processes. At phase54, only the Host crate was built and a separate test-harness server was used. The subsequent [product Host service](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/HOST_SERVICE.md) includes rx-hostd in the image and verifies it with the actual Linux clock. Physical factory connections and complete deployment administration remain future work.
