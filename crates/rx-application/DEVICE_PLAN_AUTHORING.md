# Connecting device change candidates to process authoring and compilation

phase68. Include device binding plans with current completed impact review among process-draft options, and preserve selection provenance in compile inputs and packages. Active CellConfiguration/Host bindings/Work/qualification/Runs are not changed.

## Selectable candidates

Existing GET binding-options continues to return the existing catalog of active steps. To include candidates, POST `{cell, device_plans:[{id,revision,plan_digest}]}` to the same path. This is a read and does not store settings.

P checks every plan's exact revision/digest, IMPACT_REVIEWED status, absence of unresolved issues, current builder/configuration/impact/device approval and access to all impacted cells. At most 16 plans are allowed; duplicate plan IDs are rejected. If different plans supply the same binding ID, the conflict is rejected instead of resolving by order. There are at most 512 combined candidates.

An active step with the same binding ID can be presented as the selected plan's candidate, without overwriting the active value itself. The response includes the source plan, required_cells and a separate composite catalog digest. Existing catalog digest calculation without plans is preserved.

## Saving and recovering process drafts

Specify selected device_plans in existing Save and use existing alias → binding ID selections. P recalculates the same composite catalog and preserves the exact Intent/Host/step digest. Every selected plan must be used in an actual selection; irrelevant provenance is not added.

Binding Version preserves plan refs/required cells/per-alias device_sources. Recovery of same-key results and historical binding reads also check current access to impacted cells. If a selected plan or device approval is no longer current, DEVICE_PLAN_CHANGED is returned while preserving the original snapshot. It is not silently replaced with the old active step.

Export rechecks current source/binding revision, structure, catalog, plans and authority, then recalculates selections, original step digests, actual ActionBindings and provenance and compares them with the stored version. This stage checks currentness of registry/approval metadata; it is not execution proof based on newly acquired source files. Subsequent approval/application requires actual source revalidation.

## Compile inputs v1/v2

Inputs without change candidates retain existing `rx.process-compile-input.v1` and hashes. Inputs using candidates are v2 and include, for each alias:

- Exact plan ID/revision/digest.
- Binding ID within the plan.
- Complete candidate step digest: a reference to the original source of conditions/handover policy.
- Action digest of the actual Host/Intent.

The v2 bindings_digest includes both bindings and device_sources. Inserting provenance into v1, deleting/changing provenance in v2 or changing Host/Intent fails integrity checks. At most 16 plans are allowed, and provenance aliases must exist in actual bindings.

The S compiler generates ResolvedProcess and BT XML with the actual new Intents, and preserves provenance in the compile-report too. The complete v2 input is also preserved in the process package's signed authoring/compile-input.json. These references alone do not grant S execution authority or access to P's plan registry.

## Current review and next connections

phase70 connected [device-candidate process review](DEVICE_PROCESS_REVIEW.md). Existing v1 review checks only active configuration and does not approve v2 device_sources. A new review request with explicit device_plans fixes candidate guards/configuration and revalidates sources/approvals/impact. Provenance is not removed or replaced with base-configuration guards even when step and Host/Intent match existing ones.

phase71 connected [change proposal/impact review/staging and per-Host requirements](DEVICE_CHANGE_PLAN.md). The next steps are a boundary that proves Host native/static binding changes, APPLIED_UNQUALIFIED application and qualification. Current process-change cannot actually apply device-candidate review results and explicitly rejects preparation/dispatch/application.

The existing UI decoder was extended to read/download candidate bindings saved through the API and their v2 provenance. Unresolved/stale plans block actions and show the previous snapshot. A dedicated UI for selecting new plans remains follow-up work.

Tests cover exact v2 provenance, atomic persistence/lost responses, export rejection after plan updates, provenance preservation/tampering rejection in signed packages/recompilation, and explicit legacy-review rejection. The actual JTC package/report/impact review → P draft API → S compiler/unsigned package connection was executed. See the [phase68 evidence](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase68_checks.json). The first physical cell is NOT_COMMISSIONED.
