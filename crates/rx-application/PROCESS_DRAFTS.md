# Process drafts, versions and structural validation

2026-09-11. Processes being edited are preserved separately from installed CellConfiguration/Run state. Saving is not an operating configuration change, package activation, qualification or native execution.

## Responsibilities

The P application stores current user/cell access, draft revision/CAS, request identity, content and history. S's process-authoring UI and compiler use a common source model. Structural validation code resides in `rx-process-contract::source_validation` so that P's authoring path and S's actual compiler use the same rules. P does not depend on ROS/BT/compiler execution engines.

Structural validation checks source schema, flow/node uniqueness, connectivity and reachability, cycles/recursion, finite repeats/waits, condition structure and the 4096-item expansion limit. It returns required action binding names and procedure references, but does not claim to verify actual equipment, intent, resource conflicts or authority. The S compiler subsequently continues to check actual binding existence/normalization, parallel resource conflicts and existing expansions.

Validation results record the validator name and the digest of the corresponding source/model code. This identifies which validation implementation produced a stored result. It is not an external verification package or product qualification signature.

## Incomplete documents and storage atomicity

A draft document is a canonical JSON value of at most 512 KiB. Syntactically valid JSON with incomplete source semantics can therefore be saved, with structural errors retained as the result for that version. Documents that cannot be interpreted as the Source type are recorded as format errors rather than converted into executable source.

`PreparedSave::prepare` produces document size, title, digest and structural validation results. The API performs this work on a separate CPU worker. The prepared object's internal values cannot be constructed from external wire input, and the writer rechecks the current Engineer role and terminal/cell scope immediately before commit.

One transaction stores an immutable document per content value, the latest version, version history, a small listing index, an audit event and the same-request result. Documents are preserved by cell+content digest, avoiding body duplication for each title change to identical content. Past versions continue to reference the same document.

Resending the same key and body returns the original version. Different content under the same key is a conflict. A mismatched existing revision does not overwrite current content even for a new request. Authority is checked before retrieving previous responses. API screen retries also retain the saved key/body.

## API

| Request | Purpose |
|---|---|
| POST `/api/v1/process-drafts` | Save `request_key` and `command{id,cell,expected,title,document}`. expected=null for creation; current revision for updates |
| GET `/api/v1/process-drafts?cell=...&after=...` | List within cells permitted for Engineer/Verifier. 50 entries and next ID. Source content is excluded from the list |
| GET `/api/v1/process-draft?cell=...&id=...` | Latest source and structural validation for that version |
| GET `/api/v1/process-draft?cell=...&id=...&revision=...` | Source and results for that historical version |

Updates that move an existing draft to another cell are not allowed. Reuse requires copying to a new draft ID and managing it separately in the target cell. Observer/Operator roles alone cannot read or modify undeployed drafts; Verifier can read and Engineer can save. Actual Cell/Run revisions, grants, budgets and operation outbox are unchanged.

## Editing screen

'Process design' is shown to Engineer/Verifier users. They select and connect source flows and sequence/parallel/branch/repeat/call/operation/wait/intervention nodes. The start node, child order, operation binding names, condition names and deadlines can be edited. Detailed editing of condition expressions and procedure artifacts currently uses the advanced JSON editing path.

The structural preview displays at most 256 items with bounded depth so that cycles/duplicate paths in incomplete graphs cannot expand the browser indefinitely. Complete semantic decisions follow the shared validator result for the saved version. Structural errors show their location and explanatory text while retaining the original code/message.

The editing buffer is kept in memory, bound to the current account/installation/store generation and cell. It survives menu changes and preserves source/condition JSON input before application. Other editing/saving is locked until the user explicitly applies or cancels that input. Unsaved content triggers a warning when leaving the page. This buffer is not a durable document store for restoration after browser restart, so explicit saving is required.

If a save response is lost, the existing shared pending-request path retrieves the same request. Conflicts preserve local edits, which can be compared with the current server version or a specified historical version. Comparison does not change edited content. 'Discard changes' and reopening select the latest server content. Copies are saved under a separate draft ID/revision 1.

## Boundary for handing off to packaging/execution

The screen exports the current source JSON. Exported source can be passed to the next stage using S's existing `rx-process-compile`/`compile_package` path and actual bindings. At this stage, the UI does not automatically select actual bindings or sign/deploy/activate packages. Structural success and required operation names do not constitute such approval.

Existing S compiler expansion, node/source identity, condition/resource checks and BT execution/restoration tests continue to be used. Frozen base/cell normative definitions and the four optional wire bindings are preserved. The shared SDK is exported as 80 payloads, including the new source-validation module.

## Verification and remaining scope

Tests check failures before/after saving, same-request retrieval, saving incomplete source, preservation of previous versions, concurrent updates, revocation of current Engineer authority and nondisclosure to other cells. Browser verification uses the actual P API for creation, editing, conflicts, history comparison, copying, lost-response/refresh retrieval and editing-buffer preservation. The active cell configuration must remain unchanged.

Visual selection of equipment capabilities/bindings, complete condition and intervention/recovery editors, UI connections to compiler previews and package creation/signing/deployment, change-impact analysis, review approval flows, DB paging indexes for large lists and retention policies remain outstanding. The first physical cell is NOT_COMMISSIONED.

Results were also retained from processing source saved/exported in the actual browser together with simulated bindings through the compiler in the final S image. The result is COMPILED_NOT_QUALIFIED; package signing/activation/native execution were not performed.

[Equipment operation bindings](DRAFT_BINDINGS.md) that select registered steps in the current cell and matched compile input export are connected. Creating bindings for new equipment/profiles and package approval remain separate.

## Canvas presentation metadata

The Save request optionally includes `presentation: {flows: {FLOW: {NODE: {x, y}}}}`.
Coordinates are bounded nonnegative integers. The layout is limited to 128 flow maps,
1024 positions per flow and 128 KiB overall; it never defines child order or conditions.
It is stored in the same immutable Version as the source revision and returned in
`Detail.version.presentation`. Moving a node creates a reviewed draft revision but
keeps the source document digest and executable binding content unchanged. Exported
compile input still records the latest draft revision; its entire serialized envelope
is not byte-identical across revisions. Saved-request recovery,
authorization and CAS conflicts cover the layout as part of the original request.

Omitting presentation preserves the previous layout, including when an older client
only changes title/source. Sending an explicit empty flows map clears it. Historical
versions preserve their own layout. Old records decode without presentation. The
response field is additive; clients that reject unknown response fields must be
upgraded before reading a draft with a saved layout. An old strict server refuses
those version records instead of silently overwriting them. No installed cell,
execution authority, device binding or frozen wire contract changes here.

## Library classification, archive and history

Save/Version/Summary optionally include `library: {site, service, archived}`.
Site and service are nullable, trimmed labels of at most 120 characters, not
registered operating-area identities or access grants. Omission preserves the
previous library value. Older records have no labels and are active drafts.

Archive is an ordinary immutable draft revision under the same Engineer/cell,
CAS and original-request checks. An archived draft rejects saves until a request
explicitly sets archived=false without changing its content, title, layout or
labels. The original archive request can still recover its own response. Archive
and restore never mutate the installed configuration or revoke a published run.
There is no Published state in this library metadata.

GET `/api/v1/process-drafts` also accepts optional `q`, `site`, `service` and
`archived`. Title search is trimmed, case-insensitive and limited to 120 characters;
site/service match the exact labels. Filters apply before the 50-item pagination.
Omitting archived keeps the existing all-drafts API behavior. Every read checks
current Engineer/Verifier and cell scope. The current summary scan is not a
large-library indexed query; indexed scale/retention remains a separate work item.

GET `/api/v1/process-draft-history?cell=...&id=...&before=...` returns cell, draft,
versions and next. Versions are descending summaries, bounded to 50; before is
an exclusive positive revision cursor. Use the existing exact-revision detail
read for source and layout. A historical edit is a new draft copy, never an update
to the immutable historical record. Unknown optional fields require paired strict
client upgrades as for presentation metadata.
