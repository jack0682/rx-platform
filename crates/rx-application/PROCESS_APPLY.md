# Replacing the active process configuration and preserving per-run configuration history

Status: phase51 implementation draft. Connects STAGED in the [change plan](PROCESS_CHANGE.md) to APPLIED_UNQUALIFIED. Replaces the **process configuration selected by P** based on receipts from [Host dispatch](HOST_CONFIGURATION_DISPATCH.md). It does not change robot/PLC/native driver settings or start actual operation.

## Scope and current limitations

The target is a new recipe/process/step configuration generated from a reviewed and approved process package. Only a builder that preserves the intent, Host, conditions and completion/handover rules of existing steps is used. The API does not accept arbitrary equipment definition/envelope/profile/communication settings. Impact covers the entire shared Host/resource/scope closure; neighboring cells whose process recipes do not change are also included in qualification review and epoch/fence boundaries.

The state flow is PROPOSED → IMPACT_REVIEWED → STAGED → APPLIED_UNQUALIFIED. [Requalification evidence and independent review](REQUALIFICATION_REVIEW.md) are connected. [QUALIFIED_ACTIVE and a separate start connection](QUALIFICATION_ACTIVATION.md) were added. Change cancellation/restoration and complete physical validation are future work. Successful application does not clear existing change blocks.

## Application procedure

`POST /api/v1/process-change/apply` accepts the existing `{request_key, command}` and Transition's change/cell/expected/plan_digest.

1. Check the currently registered terminal's ReleaseManager role and the intersection of user/terminal authority over all impacted cells.
2. Check the exact STAGED revision/plan, current review approval/configuration/preparation/P boot.
3. Check that all impacted Runs are COMPLETED/ABANDONED, that there is no unresolved/disputed work, no held/isolated resources and no related open cases, and that preparation fence acks exist.
4. For the same Task/preparation/request on every Host, check the APPLIED_UNQUALIFIED receipt and current boot/journal/binding/cohort/block context.
5. Outside the writer, the package worker reverifies actual Store bytes/current policy/signatures/review material. It does not accept a PASS Boolean or browser-supplied after configuration.
6. Immediately before commit, recheck 1–4, ticket boot/registration/time, and the identity of the review material/target.
7. Record configuration/history/epoch/fence/qualification/change result in one transaction.

If roles, terminal, Host context, configuration, preparation or policy change during verification, new application is rejected. If only the response is lost after commit, the same key/body retrieves the existing application record and fence IDs. Effects are not repeated with a new key or revision. Current access checks also apply when retrieving cached results.

## Ownership of recent Host confirmation

Historical observations in a durable Task alone do not allow application. The configuration worker obtains P's time **immediately before** the query/Apply call and passes it to the writer with the response. The writer recognizes only responses within 3 seconds of query start as current-process read proofs.

Proofs live in the Engine's internal memory as Task ID → `(read_started, observation digest)`. Durable receipts remain in the existing ledger, but eligibility as a recent read is not restored after restart. Refreshing the freshness of an identical repeated observation does not continually write the same Task event to the DB. Expired proofs are removed when a new proof is recorded.

Application preflight and commit check that the digest of the current durable observation matches the proof and that it is within 3 seconds on the current P clock. Separate current-state checks block RPC errors, Host generation changes, disputes and current-condition changes. ApplicationRecord preserves the per-Host request/receipt/observation digest and read start time used during application as evidence.

The 3-second value is the response age limit for **non-operating configuration selection** in this draft. It is not a duration of continuous physical stability or operating authorization. The Host's actual adapter quiescence check was performed when the earlier Host context was recorded; it is not expanded into a guarantee that physical state continued unchanged thereafter. Fresh qualification/site validation is required after application.

## Contents of one transaction

| Target | Application result |
|---|---|
| Past Run | Verify and preserve a missing initial configuration binding. Do not overwrite existing binding/result/budget/slot/evidence with the new configuration |
| Origin CellConfiguration | Replace with the reviewed after configuration |
| Neighboring cell configurations | Preserve recipes and record new epoch/scope for the full impact boundary |
| Qualification | Remove active qualification and preserve existing qualification in history and ApplicationRecord |
| mode/commissioning | SETUP. Preserve existing NOT_COMMISSIONED; otherwise REVALIDATION_REQUIRED |
| Authority/block | Increment epoch and all scopes, seal old authority, create a new latched CONFIGURATION_CHANGE and fence outbox |
| Configuration selection history | Per-cell active selection and immutable `(change, cell)` history, before/after artifacts |
| Change result | APPLIED_UNQUALIFIED revision/history/event and same-key result |

The epoch incremented during preparation is incremented once more when the actual selection is replaced. Host receipts are retained as evidence for the **preparation epoch immediately before replacement**, and are not reinterpreted as fence acks for the new epoch. Immediately after application, the new fence is in the outbox and is delivered by the existing dispatcher. Operation is not permitted without acknowledgment of the new epoch and requalification.

A storage failure rolls back all of these changes. Network/Host application and the P DB are not a distributed transaction. Host context may already be recorded while P application has failed; requerying the same preparation/request and retrieving the same application key address that state.

## Immutable per-run configuration

Creating a new Run stores a `runconfiguration/<Run ID>` binding and a content-hash-based CellConfiguration artifact together with the Run and request result. The binding connects Run/cell/configuration artifact and also compares recipe/envelope with the Run.

Existing draft DBs may contain Runs without bindings. The initial configuration may be read or a binding supplemented only if the cell has no configuration replacement history and the Run's recipe/envelope matches the initial current configuration. The first replacement transaction first creates required bindings for all existing Runs in that cell. If configuration selection history already exists but the binding is missing, it rejects with an integrity error rather than inferring it.

Read compatibility for configuration history and execution compatibility for incomplete change plans are separate. Old STAGED plans with a different builder identity are not automatically adopted by new code. Migration/restoration procedures for upgrades with incomplete Host effects have not yet been implemented.

`execution_snapshot`, `executor_artifact` and `production_view` read the configuration owned by the Run. A snapshot's cell revision/epoch/scope and access rights come from the current cell; resolved/process progress comes from that Run's immutable configuration. Historical completed Run queries do not substitute the current recipe. Whether new requests are allowed is checked separately against current state; start/execution admission requires the entire Run configuration to match the current cell configuration.

This process change does not support definition/envelope replacement, so it does not change frozen negotiation semantics. Replacing a cell definition itself in the future will require a separate design for access and negotiation policies for historical definitions.

## Query semantics

`Detail.applied=true` is the historical fact that P application of this change was recorded. If a different configuration is selected later, its difference from the current configuration is displayed as CONTEXT_CHANGED. Operating authorization is always false. APPLIED_UNQUALIFIED queries display REQUALIFICATION_REQUIRED and distinguish historical preparation proofs from post-application epoch/fences.

`host_configuration.platform_configuration_applied` likewise indicates P application history. `all_hosts_acknowledged` pertains to the existing preparation context and must not indicate operating readiness in the new epoch after application. A dedicated change UI and requalification progress/result displays do not yet exist.

## Verification

Results by scope are retained in the [phase51 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase51_checks.json). Checks cover atomicity/lost responses, the requirement for current proof and communication problems during commit, partial Host confirmation and terminal revocation, artifacts and execution/production views for past processes that actually completed and handed over, and bindings for new Runs. A separate S simulated Host and the actual P writer's mTLS path check P application and restart preservation after recovery of a lost Host response. This is evidence from test-only signers and simulated equipment, and does not replace physical commissioning.
