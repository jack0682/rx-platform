# Binding change plans for approved device actions

phase67. Revalidate currently approved device packages and persist candidate action bindings and their impact scope. Status is PROPOSED or IMPACT_REVIEWED; neither is an executable configuration. Active CellConfiguration, Work, Host bindings, grants/permits, qualification and Runs are not modified.

## Why this stage is needed

Current CellConfiguration.steps contains both device Intent/guards and their relationship to process execution steps. When a compiled process exists, its steps and host/Intent must match exactly. Replacing only device actions can create inconsistencies with process source/compiled output and the Host's static allowed Intents.

This plan therefore stores **candidate bindings without process ordering**. A candidate StepBinding has an empty predecessors array and must not be interpreted as an actual execution graph. A subsequent stage must connect candidates to process authoring/review and Host rebinding procedures. It does not erase an existing process or replace it with arbitrary ordering.

## Explicitly selected inputs

Propose takes id/cell, the exact device review ID/report revision/review digest/decision revision, a Selection for each binding ID, and a reason.

| Selection | Meaning |
|---|---|
| action | Action name in the approved catalog |
| host | Host name to bind. An unregistered new Host is recorded as an unresolved registration item |
| conditions | An explicit Condition expression for every condition ID required by the catalog |
| completion_postconditions | Current conditions to add to native success |
| handover_max_age_ns | Explicit validity period for handover evidence |

Inputs do not override an Intent's target/profile/site/calibration/resource/time limit. The exact Intent and outcome table are taken from the same approved catalog source. An ID matching a current step records the original step digest and an incremented condition revision; a new ID becomes a revision1 candidate. This does not update the active step revision.

The catalog's required condition IDs must be bound exactly. Missing/extra IDs, handover0, unknown actions and malformed expressions such as empty All/Any or reversed Range are rejected. The condition evaluator with an empty observation context is used **only for structural checks** and does not claim an actual PASS.

Facts/schemas/units absent from current FactSpec, a different site configuration, an unregistered Host and unobservable completion are recorded as unresolved issues. Missing conditions are not filled with true, and missing outcome tables do not result in invented success rules. Without a native outcome table, the Unobservable candidate and the need for separate completion/recovery review are preserved.

## Evidence and impact scope

Before each proposal and impact review, a worker revalidates current device software approval, the exact latest report/signature, and the Store source and policy. The writer rechecks current role, registration/authority, configuration, boot/30-second ticket and approval revision. Neither an approval whose source data changed nor a historical approval receipt alone can create a new plan.

Impact is calculated by adding candidate hosts/resources to all current cell hosts/resources/scopes. Related cells are selected and their shared hosts/resources/scopes are recursively included. At most 64 cells are allowed. Existing process-change impact calculation is preserved as the case with empty additional seeds.

The plan records each impacted cell's configuration digest, definition/envelope/recipe and host/resource/scope lists at that point. New candidate resources are also included in the origin's impact list. A user without access to related cells cannot commit the proposal or perform impact review. If a new shared cell appears or a related configuration changes, the previous impact-review target no longer matches current context.

## Storage, review and presentation

The before configuration is preserved in the existing immutable configuration store and referenced by the Plan. The original selection inputs, candidate, original step digest, unresolved items, approval/catalog/package provenance, builder digest and impact are combined into a Definition to calculate the plan digest. Input is limited to 128KiB, and the combined Plan and before read data to 768KiB.

Proposal, history, event and request-key result are recorded atomically. The impact reviewer must be a Verifier other than the proposer, with access to all impacted cells. Immediately before review, sources are revalidated and candidate/issues/impact recomputed and compared with the existing Definition. Impact review can be recorded even with unresolved items, but that does not resolve them or authorize application.

Impact review increments revision but preserves Definition/plan digest. Changed inputs require a new plan. Recovery of the same request returns the original record; loss of currentness does not overwrite historical facts.

Detail's context_current means agreement with current configuration/builder/impact; device_approval_current means agreement with the currently registered device approval. Neither proves that files were just revalidated. activation_authorized/configuration_changed/application_supported are currently all false.

## API

| Path | Function |
|---|---|
| POST /api/v1/device-binding-plans | Revalidate current approval and sources, then store a proposal |
| GET /api/v1/device-binding-plans?cell=…&after=… | Return 50 accessible summaries and the next cursor |
| GET /api/v1/device-binding-plan?cell=…&id=… | Return before/candidates/impact/unresolved items/currentness |
| POST /api/v1/device-binding-plan/impact-review | Independent impact review of an exact revision/digest |

Plan-editing/impact-review UI and actual application endpoints are follow-up work. Integration tests use P's development API after an actual S JTC package/report/approval flow. Current behavior and test results are in the [phase67 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase67_checks.json).

## Next connections

1. Connect candidates with explicitly resolved host/fact/site/completion items as options for process authoring.
2. Recompile/review candidates together with the existing execution order and preserve source provenance.
3. Validate Host binding changes, quiet/fence, journals/generations and current device control authority.
4. Start actual application at APPLIED_UNQUALIFIED and require revalidation/qualification and a separate user start.

This stage does not count as completing those four procedures. Original Work/evidence must retain its original configuration. Physical resource aliases/observation suitability, the production JTC provider and field acceptance remain incomplete; the first physical cell is NOT_COMMISSIONED.

phase68 added [candidate connections to process authoring/compilation](DEVICE_PLAN_AUTHORING.md). Only plans with current completed impact review and no issues are selected, and v2 inputs preserve plan/step/action provenance. An explicit rejection boundary remains so that current process review's active-step checks alone cannot approve them.
