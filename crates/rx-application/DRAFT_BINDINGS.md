# Device action bindings for process drafts

2026-09-11. This authoring path connects `Operation.binding` names in process source to StepBindings registered in the current cell. Saving or exporting bindings does not change CellConfiguration, qualification, runs, dispatch permits or native execution.

## Selection evidence and versions

Action candidates come from the current cell's StepBindings, not arbitrary code/endpoint inputs. Engineer/Verifier users see step ID, Host, target, kind, resources and intent/step digests. The candidate catalog digest binds cell/definition/envelope/site configuration and complete step definitions. Changes to conditions/completion rules therefore distinguish the selection baseline even if Intent is unchanged.

Process source and bindings have separate revisions. A binding version preserves source revision/content digest, candidate catalog digest, user-selected binding → step, original step digest and canonicalized ActionBinding. Historical selections are not changed by reinterpreting the currently named step on every read.

Saving requires current source revision, current binding revision (CAS) and the catalog digest at selection. Source structural validation must be complete; binding names absent from the source and steps absent from the catalog are rejected. Partial selections can be stored with a missing list. `complete` means that all required names have been selected.

A title-only change with the same source content digest does not needlessly invalidate existing bindings. If source content or catalog changes, the previous version is preserved but marked SOURCE_CHANGED/CATALOG_CHANGED. Saving again checks the actual current source revision and catalog.

## Atomicity and authority

The latest version, immutable history, audit event and original request result are saved in one transaction. The same key/body recovers the original binding version. Current Engineer role/cell scope is checked before the cache, and different content/stale CAS does not overwrite existing data. Recovering a save response and judging that the binding still matches current configuration are separate facts.

New bindings are limited to 128 selections and a 256 KiB stored snapshot. The draft ID bound to source/cell is checked. Verifier has read-only access.

## API

| Request | Purpose |
|---|---|
| GET `/api/v1/process-draft/binding-options?cell=...` | Current registered action candidates and catalog digest |
| POST `/api/v1/process-draft-bindings` | Store `request_key` + `command{draft,cell,source_revision,expected,catalog_digest,selections}` |
| GET `/api/v1/process-draft-bindings?cell=...&id=...&revision=...` | Latest or historical binding and currentness judgment. Latest if revision is omitted |
| GET `/api/v1/process-draft-compile-input?cell=...&id=...&source_revision=...&binding_revision=...` | Export current source and completed current binding at one cut |

Export checks both revisions exactly and rejects content/catalog that differs from current values or incomplete selections. It does not read source and bindings separately and mix them.

## One compile input

`rx.process-compile-input.v1` contains draft/cell, source/binding revisions, source document digest, bindings digest, catalog digest and actual source/bindings. Content digests are checked, but a self-digest is not a signature or package approval.

The S compiler preserves the existing two-file input and adds this input.

```text
rx-process-compile --bundle process-compile-input.json NEW_OUTPUT_DIRECTORY
```

After validating bundle digest/format, it performs existing source/intent canonicalization, expansion and parallel resource-conflict checks. Output retains authoring provenance and has status `COMPILED_NOT_QUALIFIED`. It does not execute programs or apply them to a cell.

The currently exported ActionBinding is Host+intent. This alone does not transfer an entire Cell StepBinding's completion/condition/procedure rules to a new cell. An additional path is needed to assemble/validate packages together with those definitions through the original step/catalog digests.

## UI

After saving process source, select a registered step for each action. Selection is kept separate from source editing, and the selection buffer survives menu navigation. Selections remaining only in older source are explicitly excluded; changed baselines are acknowledged with 'Review against current baseline'. Response target and request generation are compared to prevent a late response for an earlier draft from overwriting current selections.

Binding saves also use the existing pending key/body recovery path. Compile-input export rechecks server-side currentness. All bindings selected and structure verified are not presented as operating permission. Device display names are not yet provided, so registered step/Host/target IDs are shown.

## Validation and next connections

Tests verify failures before/after commit and request recovery, incorrect source/catalog/step/binding, partial selections, source/title changes, restart after registered-configuration changes, current-role revocation, matched export and bundle tampering. Registered-configuration change tests use an explicit storage fixture and do not count as release activation implementation.

Browser checks use actual APIs to verify step selection, preservation across menu navigation, lost save response/refresh recovery, export rejection after source changes, and explicit re-review/new binding versions. The registered cell remains unchanged. Compilation of the exported bundle in the final image is recorded as separate evidence.

Binding generators for new device capabilities/profiles/calibration, package assembly including full StepBindings/signing/deployment/activation, compiler preview UI integration and field acceptance remain outstanding. This authoring path reuses currently registered cell actions and does not substitute for physical validation.
