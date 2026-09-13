# Device package software verification and independent review

phase65. Record verification requests, signed reports and independent review history for imported package objects and common device catalogs. Approval scope is `DEVICE_PACKAGE_SOFTWARE`. It does not create qualification for calibration, actual control authority or physical behavior, nor cell configuration application/operating activation.

## One review target

The Request created by P fixes review/intake ID, installation/cell, package manifest/signature digest, canonicalized catalog ArtifactRef, current configuration digest, intake-policy fingerprint/source-file digest and device verification authority digest. The Job also preserves the current Store owner/registration generation.

Create is performed by a current Engineer/Verifier, and the configuration at intake must match the current configuration. If configuration changed, re-intake the source in the current configuration context. Recovering a result with the same request key returns the original Job after checking current identity/role/cell. The same review ID cannot be stored again with different content.

## Actual checks by the verification tool

`rx-device-package` provides these commands.

```text
rx-device-package validator-identity
rx-device-package review PACKAGE POLICY REQUEST OUT_DIRECTORY
rx-device-package review-signing-request REPORT KEY_ID OUT_FILE
```

It currently performs three checks on DEVICE_REFERENCE packages with a common catalog.

| Check | Actual work |
|---|---|
| CONTENT_SIGNATURE | Common verifier checks signature/publisher/permissions/contracts/target/payload/assets |
| DEVICE_SOURCE_CONSISTENCY | S's actual device decoder rechecks assembly/profile/model/release/operations/outcomes/catalog |
| CATALOG_REQUEST_BINDING | Check correlations between request package/catalog/installation/cell and the actual declaration |

If fundamental signature/request correlations are wrong, the tool refuses to create a report at all. If trusted sources were acquired but the device decoder fails, it produces a report containing a failed check and bounded issues. All three checks must be PASSED with no issues to become a software approval candidate.

The report is `rx.device-verification-report.v1`, and its scope enum represents only DEVICE_PACKAGE_SOFTWARE. Physical qualification scopes or missing checks are not deserialized as success. Report is limited to 128KiB/32 issues and includes the complete original request. The tool does not read private keys or send signing requests externally.

Intake policy is distinct from the verification tool's additional acquisition limits. The request preserves the policy fingerprint actually used by P. S uses the same policy's keys/permissions/target/assets while applying smaller acquisition limits of 8 payloads/2MiB. Applying smaller limits does not change intake-policy identity or bypass request fingerprint comparison. validator_policy_file_digest is the hash of the original policy file read by S.

The report signing message combines the `RX-DEVICE-VERIFICATION-REPORT-v1` domain, canonical key ID bytes and canonical report bytes. It does not reuse process-report or package-manifest signatures. The external signer decodes the signing request's hex into actual bytes, signs with Ed25519 and supplies verification.sig.json in the existing SignatureEnvelope format.

## P storage and approval

Device verification authority uses a separate configuration file from process authority. Its schema is `rx.device-verification-authority.v1`, with allowed validator digests declared for each key ID/public key. File pins and semantic digest are checked at startup, and report/approval workers reread the file. A package cannot install its own verification key.

The P worker revalidates the original package from the registered Store under current policy and checks report request/catalog bytes/verification signer/validator. File/cryptographic work runs in the existing bounded worker under a single semaphore. Its result is passed to the writer as a Prepared token that cannot be deserialized.

The writer rechecks current role/cell/configuration/registration/authority/boot/30-second ticket and expected revision. It records report/signature/version digest/history/event/same-key result in one transaction. A new report version is not automatically linked to an earlier approval. Historical versions remain readable.

APPROVE requires the Verifier role and an account other than the package submitter. It requires the exact latest report revision/review digest, current checker digest and all software checks passing. Immediately before approval, the worker revalidates package/policy/authority/signature; immediately before writer commit, the same context is checked again. Approval does not proceed if policy/report/role changes in between. REJECT can be recorded even after current authority is revoked, provided the latest target is explicitly identified.

Decision, history/event/request-key result are recorded atomically. After a lost response, the same request recovers the original decision. If policy is subsequently revoked, recovering a historical decision does not restore current approval. A read's approval_matches_current_review means agreement with currently registered context/latest report, not that actual files were just revalidated. Subsequent configuration application must require separate current-source validation.

## API and deployment boundary

| Path | Function |
|---|---|
| POST /api/v1/device-reviews | Create Job and verification request |
| GET /api/v1/device-reviews?cell=…&intake=… | Read pages of 50 with next cursor |
| GET /api/v1/device-review?cell=…&id=…&revision=… | Latest or historical report/decision/currentness |
| POST /api/v1/device-review/reports | Intake using file path/report digest/expected revision |
| POST /api/v1/device-review/decisions | Approve/reject an exact version |

All mutations use the existing request_key wrapper and current authentication. HTTP and terminal HTTPS invoke the same application. No new gRPC service or frozen wire contract was added. package-intake-context supplies a separate device_review_authority_digest.

Specify a pinned file in rx-platformd's package_intake.device_review_authority. Without one, device review Jobs cannot be created. The development-only local service has the same optional field, but it is not enabled as production trust by default. Browser UI for changing review state is follow-up work; integration tests in this stage invoke actual APIs.

Actual validation results follow the [phase65 evidence](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase65_checks.json). This stage does not hardware-attest the device validator's code/signing supply chain; its trust boundary is that a registered trusted signer signs the designated tool's results. JTC production Authority/lifecycle/fencing, device review UI, cell configuration changes from approved actions, qualification and physical acceptance remain outstanding. The first physical cell is NOT_COMMISSIONED.

phase66 connected the [device review UI](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/apps/operator/DEVICE_REVIEW_UI.md). Lists return 50 Summaries instead of complete reports; detail/historical-version APIs remain available. Existing phase65 list consumers must update to summary format. The UI uses confirmation dialogs bound to the current version and existing pending-request recovery. Actual configuration changes/physical qualification remain follow-up work.

phase67 added [action binding change plans](DEVICE_BINDING_PLAN.md) that revalidate current approvals/sources. They record conditions/Hosts/candidate-resource impact and independent review without changing actual configuration/qualification/Runs. An application procedure connecting candidates to process revalidation/Host binding changes remains follow-up work.
