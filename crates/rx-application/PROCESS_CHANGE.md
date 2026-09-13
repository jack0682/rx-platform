# Change planning, impact review, staging and application preparation for approved processes

2026-09-12. This implementation connects SR03's `PROPOSED → IMPACT_REVIEWED → STAGED` and the subsequent application preparation. **It does not yet change the current installed configuration.** `APPLIED_UNQUALIFIED → QUALIFIED_ACTIVE` requires connections to Host configuration acks, disposition of residual work/support, preservation of previous execution references and fresh qualification.

This is an actual change path that prevents a button on an approval screen from connecting directly to an execution-selection pointer. Through Stage, operating authority is unchanged; impacted cells' epochs/permits/fences are handled only when a ReleaseManager on a registered terminal explicitly requests application preparation.

## Building the target configuration from an approval result

A proposal pins the process review ID, verification revision, review digest, approval decision revision and reason for change. A new proposal can be created only after the package worker rechecks the original Store, policy, verifier authority, signed report, source and result. P's latest software approval must also remain valid.

The target configuration is not an arbitrary new CellConfiguration accepted from a client and stored. It is built from the reviewed configuration snapshot and the actual verified resolved process.

- Connect each compiled operation node's binding to the existing StepBinding selected during review.
- Copy existing operation rules, including host, normalized intent, execution conditions, completion rules, condition ID/revision and handover age.
- The new StepBinding ID is the compiled node ID. Using the same template in multiple positions/repetitions separates it into each node ID, while preserving the original template ID in `step_origins`.
- After source order is checked in non-operating review, responsibility for the static predecessor list is transferred to the compiled tree. It is explicitly converted into the form required by P's existing configuration validator. Independent checks of the signed source/result and predecessor order run in the preceding review path and the reverification worker.
- Replace the recipe reference with the actual resolved process SHA-256/schema/size and include the process. Preserve all other cell configuration fields.
- Preserve before/after as separate immutable configuration documents addressed by content hash. Reject identical configurations, configurations with no equipment operations, or expanded configuration records exceeding the current 1 MiB limit.

The existing definition/envelope is not promoted to a new qualification. References preserved at this stage represent existing hardware/constraint context; the impact of process-order changes on the envelope and verification scope must be reviewed again. There is no implementation that attaches the existing qualification to the new configuration and executes it.

## Impact scope and review

The base unit is the entire cell. Starting at the change origin, include cells sharing any of the following until reaching a fixed point:

1. scope
2. command/support resource
3. Host

Cells are included even if only the Host matches and scope/resource names differ, to avoid missing effects through a common process/control path. The current closure limit is 64 cells. It does not assume away unknown dependencies and treat only some nodes as unaffected. The current builder is not a general changer that adds equipment/Hosts/layout; such changes must extend the impact graph.

Pin each impacted cell's complete configuration digest, definition/envelope/recipe references and Host/scope/resource lists. Explicitly require qualification/recovery review and Host configuration confirmation. Distinguish this static impact context from current blockers in running Runs/operations/resources/cases. Normal work progress alone does not change the proposal's static digest.

Current access to all impacted cells is required for proposal, query, review, staging and preparation. Authority over one origin cannot retrieve other cells' change information or revoke their authority.

A Verifier account different from the change proposer reviews the reason for change, before/after, scope, and verification/recovery impact. It specifies the plan digest and revision and records comments. Changes to impact review also retain CAS and history. A ReleaseManager requests Stage, which rechecks package/report/current approval and target regeneration.

## Effects of application preparation

A ReleaseManager on a currently registered terminal explicitly requests `BeginPreparation`. This API does not mean actual application is complete.

The following occur in one transaction:

- Reject if preparation for another change is in progress within an overlapping impact scope.
- Recheck current review/approval, plan digest/revision, builder identity and all impacted configuration digests.
- Record new epochs and scope epochs across all impacted cells and add a latched `CONFIGURATION_CHANGE` block.
- Seal existing pre-entry permits/queues and live mandates under existing invalidation rules. Operations for which actual entry cannot be ruled out retain UNKNOWN/unresolved state and resources.
- Handle Runs, start attempts and native dispositions under existing invalidation rules as well. Do not turn existing conclusions or evidence into new success.
- Put fences for relevant Hosts in the outbox and bind each message ID/target epoch/scope to Preparation. Record Preparation, change revision/history/event and request result together.

The same request key/body retrieves the initial preparation result. Lost responses do not cause duplicate new epochs/fences. If a Runtime restart or another hold makes the preparation's boot/epoch stale, the query displays staleness. Repreparation requires explicit `refresh=true` and the latest change revision. Previous preparation history, existing blocks, operations and resources are not deleted.

There is currently no API that automatically cancels a change under preparation or restores previous operating authority. Reconciliation/cancellation also require consideration of remaining physical state and appropriate recovery procedures.

## Queries and unmet conditions

Queries return stored before/after and current blockers. They distinguish at most 256 items from the total count/truncation status.

- Current configuration/approval or preparation boot/epoch has changed
- Runs not yet disposed
- NONE/UNRESOLVED or disputed operations
- Held/isolated resources
- Open cases
- Missing Host fence acknowledgment for preparation messages
- Host configuration acknowledgment required

Fence confirmation compares the exact preparation message/epoch/scope with the currently registered Host boot/journal. **Fence confirmation is not a substitute for configuration application acknowledgment.** The Host process-context service/receipt and P transport client are implemented in the [Host contract](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/PROCESS_CONFIGURATION.md). The [P durable coordinator](HOST_CONFIGURATION_DISPATCH.md) connects explicit Batch requests, automatic dispatch and receipt incorporation. `HOST_CONFIGURATION_ACKNOWLEDGEMENT_REQUIRED` remains until Host confirmation matches the current preparation. Without sufficient Host confirmation, the after configuration is not installed and neither `APPLIED_UNQUALIFIED` nor `QUALIFIED_ACTIVE` is recorded.

Through STAGED, `applied=false` and `activation_authorized=false`. After [P application](PROCESS_APPLY.md), APPLIED_UNQUALIFIED and `applied=true` are recorded, but operating authorization remains false. Having few remaining blockers does not mean physical stopping/support disposition has been confirmed. Per-Host receipt status—applied/not applied/unconfirmed—and agreement with current preparation are displayed separately. Even after all Hosts confirm, P application and qualification remain subsequent steps.

## API

Uses the existing browser BFF `{request_key, command}`. A dedicated change screen is not yet provided.

| API | Role and content |
|---|---|
| POST `/api/v1/process-changes` | Engineer: propose a change with review/approval references and a reason |
| POST `/api/v1/process-change/impact-review` | Separate Verifier: target(change/cell/expected/plan_digest) and comments |
| POST `/api/v1/process-change/stage` | ReleaseManager: reverify the reviewed target to reach STAGED |
| POST `/api/v1/process-change/prepare` | ReleaseManager on a currently registered terminal: prepare application using target and refresh selection |
| POST `/api/v1/process-change/configure-hosts` | ReleaseManager on a currently registered terminal: explicitly approve a durable Host request Batch |
| POST `/api/v1/process-change/apply` | ReleaseManager on a currently registered terminal: replace P configuration with APPLIED_UNQUALIFIED after fresh Host confirmation and package reverification |
| GET `/api/v1/process-change?cell=...&id=...` | Engineer/Verifier/ReleaseManager: query before/after and blockers after checking authority over all impacted cells |

Proposal/stage worker tickets bind the current boot/registered Store owner/policy and are valid for only 30 seconds. Current roles and impacted-cell authority are checked even before retrieving cached results. Different intent under the same key, stale revision/plan digest, and revoked software approval are rejected.

CONFIGURATION_CHANGE is an internal block reason and projects to EXTERNAL_RESTRICTION in the existing wire CellReason. The UI also displays it as 'Configuration change preparation'. Frozen base/cell normative files and enum values were not changed.

## Next connections

1. Post-application fence acknowledgment for the new epoch and Host requalification.
2. Reconciliation and interruption/cancellation policies distinguishing actual not-applied/applied/unknown states and mixed configurations.
3. Preservation of immutable Run/evidence configuration references when cancelling/restoring an applied configuration.
4. Retention/revocation of old qualification in APPLIED_UNQUALIFIED and fresh qualification procedures.
5. UI for change review, preparation, progress and unmet conditions.

The work above is not considered complete by completion of this staging/preparation implementation. Physical equipment remains NOT_COMMISSIONED.
