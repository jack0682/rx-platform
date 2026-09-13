# Software review of processes containing device change candidates

phase70. Compare action candidates created from approved device packages with process-package provenance and record an independent process software review decision. Cell configuration, Host settings and operating authority are not changed.

## Fixed context

Create's optional `device_plans` is a list of exact IDs/revisions/plan digests. The same catalog builder as existing process drafts checks IMPACT_REVIEWED, no issues, current device approval/configuration/impact scope and access to all impacted cells. Selected plans must be used in actual binding selections.

With candidates, the request is `rx.process-review-request.v2` and includes `device_context_digest`. The Job's `device_context` snapshots the catalog digest and each plan/original device review Job/report version/independent decision. The limit is 16 entries, sorted by ID, with no more than 512 KiB of complete canonical bytes. The request digest and signed S report bind this snapshot digest.

Existing v1 remains without context fields. Adding a context digest to v1 or omitting it from v2 is rejected. The base configuration is not overwritten with a candidate configuration; a separate configuration combining candidate steps is calculated only during independent checks. Preserving the original configuration retains before evidence for the later actual application procedure.

## Source revalidation at intake and approval

The worker revalidates the process package and all device packages under the same Store owner and current policy. It checks the separately pinned device authority file, signatures, validator and report digest. A historical DEVICE_PACKAGE_SOFTWARE approval does not justify skipping source revalidation.

P regenerates candidates from verified device catalogs and original selection inputs. Conditions, completion outcome mappings, postconditions, Host/Intent, condition revision, handover validity and previous step digest must match the stored plan candidates. Regeneration uses the same function as initial plan creation.

Within the process's signed compile-input, per-alias plan ref/binding ID/step digest/action digest are compared exactly. The composite catalog is also recomputed from the original configuration, sorted plan refs and combined steps. Missing/changed provenance or substitution of another candidate is not allowed. Existing source/resolved structure, authority, condition and predecessor checks continue against the candidate configuration.

Immediately before commit, the writer again checks current plan revision/content, builder, approved report/decision, authority, configuration/transitive impact/registered policy/account permissions and the ticket's boot/30-second lifetime. Access to all impacted cells applies not only to creation/report/decision but also to historical reads/same-key recovery. Lists do not expose Jobs whose dependent context is inaccessible.

When device approval is revoked by a new decision, the recorded process approval and report remain historical material, and `context_current`/`approval_matches_current_review` become false. Recovering a historical decision with the same key does not reissue it or restore current approval.

## Application and UI

Approval scope is PROCESS_PACKAGE_SOFTWARE. phase71's [device change plans](DEVICE_CHANGE_PLAN.md) connect candidate proposals, impact review, staging and per-Host requirements. Actual preparation/dispatch/application are blocked. This boundary must be extended after connecting Host/native setting changes and operating envelopes, quiet/fence/journal generations and post-application qualification. activation_authorized is false.

The existing operator UI decoder preserves v2 requests and candidate context and rejects data missing the candidates. It presents candidate data as review evidence and explains that current cell settings are unchanged. A dedicated authoring screen for selecting new device plans is follow-up work; creation in this stage uses the API path.

## Verified scope

Actual signed JTC device packages, process packages and S compiler reports were imported into the P API, and independent approval was verified. Accounts with access to only some cells, submitter self-approval and changed device authority files are rejected. After device approval revocation, tests verified loss of process currentness, rejection of new approvals and recovery only of the same historical request. Complete source boundaries and logs follow the [phase70 record](https://github.com/jack0682/rx_docs/blob/codex/initial-draft/references/implementation/phase70_checks.json).

These results do not establish physical validation, a production signing service, a JTC operating provider or field application acceptance. The first physical cell is NOT_COMMISSIONED.
