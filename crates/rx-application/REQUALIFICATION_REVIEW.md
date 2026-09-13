# Post-application requalification requests, evidence and independent review

Status: phase52 implementation draft. After [P configuration application](PROCESS_APPLY.md), this pins verification material, checks actual bytes/signatures/current context and records independent review decisions. Approval scope is `REQUALIFICATION_EVIDENCE_REVIEW`. It does not create Qualification/QUALIFIED_ACTIVE, clear Host blocks or start Runs.

## Responsibilities and trust boundary

The policy owner supplies acceptance plans per exact configuration, required items, dependencies, constraints and authorized verifier public keys. A ReleaseManager on a registered terminal pins the verification scope with new epochs/fences. External verifiers create and sign per-item results and evidence; an Engineer imports them. A Verifier different from both requester and importer reviews them again. A subsequent qualification activation path must reconnect this approval to current Host/equipment context.

A signature is an authorized verifier's assertion. Signature checks or the mere presence of six areas do not establish technical sufficiency of testing or honesty of measurement. Specific tests, risks, expected results and allowed combinations are the responsibility of the reviewed acceptance plan/criterion specification. GOOD signals or human checkmarks do not replace actual protective-function verification.

## Policy and required scope

Policy v1/v2 is startup-pinned JSON. v2 purpose scope and actual issuance/activation follow [qualification activation](QUALIFICATION_ACTIVATION.md). Configure it through `package_intake.qualification_policy: {path, sha256}`. The public API does not install policies/keys. File pins are checked at startup and on each worker task, without hot reload.

| Object | Fields and responsibilities |
|---|---|
| Profile | cell, exact complete configuration ArtifactRef, definition/envelope, environment, acceptance_plan, limitations, dependencies, criteria |
| Criterion | Unique ID, area, immutable specification ArtifactRef, allowed evidence_schema |
| Key | key ID/public_key, allowed validator digest set, allowed SIMULATION/PHYSICAL environment set |

Each profile explicitly includes all six areas: SOFTWARE, EQUIPMENT, CELL_INTEGRATION, RECOVERY, PROTECTION and OPERATIONS. An area may contain several items, and every item is required. A conclusion that the scope needs no separate protective action is itself subject to review of its specification and evidence; it does not remove a required area.

It references the exact complete configuration hash rather than inferring a Cartesian product of maximum values. The material set must include cell definition/envelope/recipe/site configuration, per-step profile/site/calibration and trajectory/tool/program/parameter/mode-transition/stream profile hashes. Configuration/definition/envelope material is also verified as actual files. Without originals matching placeholder hashes, report verification cannot pass.

The policy semantic digest normalizes the ordering of keys/profiles/criteria/dependencies. The file pin is the hash of original bytes. Even with identical semantics, a changed file is rejected by the current worker. Actual policy/key updates and deployment, and their connection to active qualification revocation, are future work.

## Requests and generations

Begin accepts a new request ID, applied change ID/origin cell, exact change revision, an expected_cells revision map for all impacted cells, and the registered policy digest. Missing/extra cells are rejected. The intersection of current terminal/user scope must include every impacted cell. The change must be APPLIED_UNQUALIFIED, and the selected configuration must match the application record; active qualification is not overwritten.

Invalidation of all impacted cells updates epochs/scopes/latched blocks and records fence outbox/Request/Job/user request result in one transaction. It does not wait on the network. The same key/body retrieves the same request/fences. A new request changes the generation, making the previous review no longer current.

Request pins all profiles, application-record digest, Runtime boot, each cell's revision/epoch/scope/blocks and exact fence IDs. Currency of a previous request is not restored after restart. If the applied configuration is retained, a new requalification request can be created with the current terminal and revision.

## Reports and originals

```text
<import_root>/<directory>/
  qualification.json
  qualification.sig.json
  artifacts/<sha256>.bin
```

`rx.requalification-report.v1` contains the complete original Request, validator digest and checks per `(cell, criterion)`. Each result has PASS/FAIL/NOT_RUN, an explanation and an evidence ArtifactRef array. Missing/extra/duplicate items are rejected. FAIL also requires evidence. Unperformed work is explicitly NOT_RUN; keeping only PASS results cannot hide other tests.

The signing message consists of the following bytes, without a final newline.

```text
RX-REQUALIFICATION-REPORT-SIGNATURE-v1\n<key ID>\n<report digest>
```

The report digest is the `RX-REQUALIFICATION-REPORT-v1` canonical digest. Package/process review signatures are not reused. The original request, current policy, signer/validator/environment and all profiles are compared.

The worker actually reads required documents/dependencies and every result's evidence, checking hash/size/schema references. Expert interpretation of physical performance, units and test design inside the material has not been implemented as a generic parser. This stage checks the exact signed material set.

Each file is at most 8 MiB, and unique originals total at most 32 MiB. Report/signature/policy metadata each use the existing 1 MiB limit. PackagePath/capability-based reads reject path escapes/symlinks. Each worker permits 1 concurrent filesystem/crypto task, and the writer does not open files directly.

## Atomic storage and independent review

Originals are stored as base64 records in 256 KiB chunks and a complete blob manifest. Reads recompare chunk hashes and complete hash/size. Splitting accommodates the 1 MiB DB document limit without splitting the transaction. Original chunks/manifest/report version/history/event/request result commit together. Chunks conflicting with existing ones are not overwritten.

The Version digest binds report/signature/recorder/time/revision/check results. Every result must be PASS for `ready_for_review=true`. FAIL/NOT_RUN material is retained but not displayed as ready for approval.

Approval conditions are:

1. Current Verifier authority and access to all impacted cells. An account different from requester and importer.
2. Latest report revision/digest and exact previous decision revision.
3. Matching current policy/Runtime boot/application record and cell revisions/epochs/scopes/blocks.
4. Acks for every preparation fence ID/epoch/scope against current Host boot/journal.
5. A private PreparedDecision from worker reverification of stored originals and the current pinned policy.
6. Rechecks of roles/context/version/CAS at commit.

A Hold, restart or configuration/authority/policy change during review prevents new approval. A lost response retrieves the original decision using the same key. Importing a new report version does not make previous approval apply to the new material. Because rejection grants no new permission, it is recorded after checking current target/role/CAS without requiring the crypto worker again.

## API and display

Reuses existing authentication, CSRF, terminal BFF and `{request_key, command}`.

| Path | Role and content |
|---|---|
| POST `/api/v1/qualification-reviews` | ReleaseManager on a registered terminal: Begin and new restrictions/fences |
| GET `/api/v1/qualification-review?cell=...&id=...` | Engineer/Verifier/ReleaseManager: Job/latest Version/Decision/currency |
| POST `/api/v1/qualification-review/reports` | Engineer: import by report digest/relative directory/expected version |
| POST `/api/v1/qualification-review/decisions` | Independent Verifier: approve/reject an exact version digest |
| POST `/api/v1/qualification-review/artifact` | Read-only `{review,cell,reference}`. Return octet-stream only for originals referenced by a historical version of that review |

GET separates context_current, fences_confirmed, approval_current and activation_authorized=false. Current policy means the semantics registered in the Runtime; it does not mean files are continuously monitored. The worker rechecks the original policy pin on every new import/approval. Original-material reads also enforce current authority and scope over all impacted cells. Arbitrary paths or generic blob URLs are not accepted. Lists, historical-version selection, dedicated review UI and policy preparation tools are future work. The current JSON model is an application-owned v1 artifact; it did not change base/cell normative definitions or optional gRPC bindings.

## Subsequent qualification activation

The approval digest must be bound to a new qualification ID/revision/configuration/current epoch, and durable receipts of acceptance by each Host must be collected. P records QUALIFIED_ACTIVE after reconciling partial acceptance/unknown states/restarts. Even then, Run start remains a separate procedure involving user intent, current conditions, Host preparation and mandate/permit.

The review path itself is not activation authority. [Separate issuance/activation](QUALIFICATION_ACTIVATION.md) requires v2 policy, original reverification and Host acceptance. Test-fixture keys/signers/material are for simulation protocol verification and are not installed as default product trust or site evidence. The first cell is NOT_COMMISSIONED.

## Verification and limitations

Follow the [phase52 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase52_checks.json). Ledger tests cover preparation/import atomicity/lost responses, omissions/forgery/environment mismatch, NOT_RUN, independent review/fences/change races, version replacement and splitting/rollback/retrieval of a 2 MiB original. An actual P writer plus a separate S simulated Host checks the folder worker, rejection of file corruption/policy changes, Arm rejection after approval and loss of approval currency after restart.

Report import, independent approval and original/status reads were additionally verified on an actual loopback HTTP server. Begin requests without terminal identity were rejected. The positive Begin path was checked with a registered-terminal identity in the actual writer; dedicated terminal HTTPS/browser UI tests for the new paths are future work. No claim is made that actual equipment/process/protection tests across all six areas were performed.

Writer occupancy/impact on other cells during 32 MiB intake, long-term history scale and load from multiple concurrent users have not yet been measured. Capacity limits must not be interpreted as actual operating performance guarantees.
