# RX Cell Operations Contract v1.0

Status: v1.0 document revision 2026-09-14-english.1 · Normative design document. Not physical-operation authorization, safety-function performance, or a certification decision.

This contract defines cell **operating conditions, execution authorization, operator intervention, recovery, and revalidation after change**. It extends the [common operation contract v1.0](https://github.com/jack0682/rx_docs/blob/main/docs/contracts/v1.0/README.md) without retroactively changing existing acceptance, outcome, uncertainty, or duplicate-handling semantics.

| Document | Role |
|---|---|
| [01 Scope, conditions, and functions](01_scope_conditions_functions.md) | First-cell boundary, operating scope/observations/material support, hazard/safety-function responsibilities |
| [02 Authorization and invalidation](02_authorization_invalidation.md) | Automatic-run start intent, single-operation permits, revocation/races/restart |
| [03 Intervention, recovery, and change](03_intervention_recovery_change.md) | Operator intervention/access evidence/handover/recovery paths/validation impact |
| [04 Data, integration, and UI](04_protocol_integration_ui.md) | Types/APIs/atomicity/base-contract integration/mixed versions/operational UI |
| [05 Counterexamples, validation, and completion audit](05_validation_audit.md) | SR01–03 and race traces, implementation/physical-validation obligations, per-requirement completion decisions |
| [Research and decision evidence](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/cell_operation_research_2026-09-10/research_and_decisions.md) | Original-source facts, RX policies, rationale for selecting/rejecting alternatives |

## Principles

- `MUST/required/prohibited` denotes implementation requirements. Separately validated profiles provide model-specific values, signals, and procedures.
- Normal automatic runs may repeat within valid scopes/current conditions. No human click is required for every operation.
- A simple lost communication response is not itself a safety stop or human intervention. If original intent/authority remain and results are recoverable, reconcile by querying the same operation.
- After human access/manual-state change, safety stops, or device/authority generation changes, do not revive prior start intent. Explicit revalidation and a new start are required.
- RX dispatch permission does not replace actual protective-function conditions. If RX/Host/OS participates in that function path, include it in required performance, fault assumptions, and validation.
- Preserve the Rust/ROS boundary and two RX repository/container structure. Determine device support through per-profile validation. Do not assume a new mandatory central safety server or third repository.

## Scope

Applies to a single Runtime and device Hosts on the same primary computer and an explicitly defined operational cell. Profiles represent stationary/wheeled/legged model differences. Installations with unresolved actual equipment, facility controllers, signals, or intervention procedures are `NOT_COMMISSIONED`. Contract design can be finalized in this state, but production/site-access permission cannot be issued.

This edition turns the N01–N06 analysis into concrete rules and traceability tables. Protocol extension is `rx.cell.v1`, reusing `rx.contract.v1` as the base. Relevant cell releases must support both; no automatic downgrade to old Hosts. Detailed compatibility rules are in 04.

[Independent documentation review record](review_record.md) · [Contract manifest](protocol_manifest.json) · [Integrity record](manifest_integrity.json)

Vendor-neutralization scope, compatibility, and hash changes are in the [documentation revision record](revision_2026-09-14.md). Subsequent translation-only byte/hash changes are in the [English translation revision](translation_revision_2026-09-14.md). Do not present the 2026-09-10 review as new execution validation of these revisions.
