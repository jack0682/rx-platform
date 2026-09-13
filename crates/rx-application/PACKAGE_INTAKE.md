# Package intake for each user and cell

2026-09-12. `package_intake` binds a verified, stored package to the current user and cell context and admits it to the ledger. The result is `AWAITING_REVIEW`. The receipt identifies the content, submitter and target cell; it is neither independent review approval nor operating activation.

The prerequisite is the [package store](../rx-package/STORE.md). The application does not make the writer wait while reading files or verifying signatures. A separate Runtime worker performs those tasks, and the application checks the actual ownership of its result and the current authority and context.

## Processing sequence

```mermaid
sequenceDiagram
    participant U as Configuration client
    participant A as HTTP handler
    participant P as Platform writer
    participant W as Package worker
    participant F as Dedicated file store
    U->>A: Intake intent and request key
    A->>P: Preflight with current identity
    P->>P: Check current user, terminal, cell, configuration and policy generation
    P-->>A: Non-serializable verification ticket
    A->>W: Ticket verification task
    W->>W: Reacquire pinned policy file; check pin; verify dependencies/assets
    W->>F: Store and reacquire bytes after verifying the source
    F-->>W: StoredPackage bound to store owner and policy fingerprint
    W-->>A: Prepared result
    A->>P: Commit intake
    P->>P: Recheck current identity, configuration, policy and expiry
    P->>P: Record receipt, event and request result in one transaction
    P-->>A: AWAITING_REVIEW receipt
    A-->>U: Intake response
```

The HTTP handler coordinates calls to the writer and worker. ticket/Prepared/StoredPackage are internal Rust values and are not exported in HTTP responses. The writer can process other commands while HTTP calls the worker.

1. The client reads the current intake context. The configuration fingerprint binds the entire CellConfiguration; the policy generation binds the registration of the package service configured in the current Runtime.
2. The POST contains the intake ID, cell, title, relative path below the import folder, expected object ID, and the previously observed configuration digest and policy generation. Both manifest/signature hashes of the object ID are pinned. Replacing the folder contents with a different signed package does not allow it to be accepted under the same request.
3. The writer checks the current session/user/terminal scope and Engineer role. If the same request key/body has already committed, it retrieves the existing receipt. This path is **retrieval of a past intake result**, and is not presented as new verification or approval. A different body with the same key is a conflict.
4. A new request must match the current service registration and configuration digest. An existing intake ID is rejected. A ticket valid for 30 seconds is issued. It holds the input, request key, actual Identity, Runtime boot, service registration and issue/expiry times, and does not implement Deserialize. This ticket is not yet a durable intake record.
5. The worker permits only one acquisition at a time. If an acquisition is running, it rejects additional work instead of accumulating an unbounded wait queue. It does not occupy the writer queue for other business commands. Cancelling a task does not guarantee immediate cancellation of filesystem I/O already in progress.
6. For every new request, the actual daemon worker rereads the pinned policy file and checks its exact bytes hash. The policy loader also rechecks dependency/asset inputs. It verifies the source package, compares both requested hashes, and then stores it in the Store. Only bytes reverified and reacquired from the store are included in Prepared.
7. At commit, it rechecks the current identity/role/terminal, Runtime boot, service registration, CellConfiguration digest and ticket time. If authority, terminal or policy generation has changed, or the ticket has expired, it creates no new intake record. Files may remain as unreferenced objects.
8. It records the receipt, `rx.event.package-intake-submitted.v1` and idempotency result in the same SQLite transaction. The status is always AWAITING_REVIEW; it does not change Cell/Run/qualification/permit/Host outbox.

## Boundary between file storage and DB commit

The filesystem and SQLite are not described as one transaction. The order is **publish a complete object → writer intake commit**.

| Interruption/failure point | What may remain | On retry |
|---|---|---|
| Before/after initial authority and context checks | No receipt | Recheck from the current conditions |
| Source verification failure | No receipt | Retry the same intent after checking input and policy |
| During file writing | Incomplete `.incoming-*` | No automatic intake/replay. Existing Store rules apply |
| After a complete object, before DB commit | Unreferenced object | New intake is possible after comparing all bytes and reverifying |
| DB transaction rollback | No receipt, event or request result | Intake can be retried with the same key |
| Response lost after DB commit | All three records exist | Retrieve the original receipt after checking current identity/scope |
| Source file changed/deleted after commit | Original receipt and stored object | The same request retrieves the original result. A new request verifies the source afresh |

Historical retrieval does not assert that the content remains valid now. Inability to retrieve a past receipt and inability to approve new use are different issues.

## Actual verification results and current policy

The fields of `StoredPackage` are private, and only the Store constructs it. The internal value binds:

- The owner ID of the Store instance that actually read the bytes.
- The semantic fingerprint of the VerificationPolicy actually used for verification.
- The manifest/signature object ID.
- Immutable VerifiedPackage bytes and manifest.

Even identical content obtained from a Store other than the registered one cannot be used as the result of that ticket. Nor can a result verified under a different policy pass merely by attaching the correct policy ID string. The application checks this both when constructing Prepared and at final commit. Store and policy configuration itself belongs to the trusted Runtime assembly boundary. There is no API through which a browser can change the service registration by submitting a public key, policy fingerprint or success Boolean.

The policy semantic fingerprint includes publisher/key/allowed kinds and permissions, both contract hashes/ABI/target, verified dependency manifests/signatures, asset references and acquisition limits. Integer limits are normalized to the existing Counter string representation so that rounding large u64 values cannot produce the same fingerprint. This differs from the policy file digest. The file digest pins input bytes; the semantic fingerprint pins the actual verification policy.

`configure_package_intake` is an internal assembly command. Configuration/replacement/disablement is recorded in the ledger and uses a new generation ID. A registration from a previous boot is not automatically activated after restart. The new daemon verifies current files, then creates a new Store owner and registration generation. Previous intake history is retained, but is not automatically promoted to current review context.

Hot-editing the policy file is not a normal trust-change procedure. Deployment and revocation of production trust revisions remain future administration features; the current implementation uses startup configuration pins and explicit internal registration/disablement. The actual worker checks the file pin on every new acquisition. The writer does not monitor external file tampering after an acquired policy snapshot through synchronous filesystem reads. A normal policy change must replace/revoke the registration generation so that pending commits are rejected as well.

## HTTP contract

This currently extends the browser BFF. The frozen base/cell Protobuf and 8 normative documents were not modified.

| API | Authority | Result |
|---|---|---|
| GET `/api/v1/package-intake-context?cell=...` | Engineer or Verifier for the cell | Configuration digest, registered policy generation or null |
| POST `/api/v1/package-intakes` | Engineer for the cell | New intake or historical Receipt for the same request |
| GET `/api/v1/package-intakes?cell=...&after=...` | Engineer or Verifier for the cell | At most 50 entries in ID order and a next cursor |
| GET `/api/v1/package-intake?cell=...&id=...` | Engineer or Verifier for the cell | View compared with current context |

The mutation body follows the existing `{request_key, command}` format. Submit fields are `id`, `cell`, `title` (1–120 characters), `relative_path`, `object`, `configuration_digest` and `policy_generation`. Unknown fields, path escapes, backslashes/absolute paths and similar input are rejected by the existing strict JSON/PackagePath rules. Adding a field such as `approved=true` cannot change the semantics.

The Receipt preserves the submitter, actual terminal ID, submission time, object/manifest/target cell/configuration/registration. The GET View explicitly states:

- `review_context_current`: Whether the registered service generation and cell configuration match those at intake. **This is not a current verification result for files or qualification.**
- `content_reverification_required=true`: Fresh content verification is required for review/use.
- `activation_authorized=false`: Intake/retrieval is not operating authorization.

Current authority is checked even before returning cached results. A receipt cannot be obtained by looking up the same ID under a different cell. A user with only controller/Host roles cannot substitute for Engineer authority. History can be retrieved when the service is unconfigured, but new intake is unavailable.

The current worker collapses input/verification/acquisition errors and busy into `PACKAGE_VERIFICATION_FAILED` 409. Classifying actual causes and connecting them to an operational support screen is future work. Lost writer-commit responses retain the existing `outcome_unknown` rule. In that case, retrieve the result with the existing key/body instead of immediately substituting a different request key.

## Platform startup configuration

The optional `package_intake` field was added to `rx.platform-startup.v1`. Omitting it leaves the worker unconfigured.

```text
package_intake:
  import_root: /mnt/rx-import
  policy:
    path: /etc/rx/package-policy.json
    sha256: <SHA-256 of the exact file bytes>
```

This describes the fields and is not a valid JSON example. The policy uses the existing `rx.package-verification-policy.v1` format. Working configuration requires absolute paths and actual hashes. Dependency/asset paths are also absolute. The import root must be an actual directory and must not overlap the authority data directory. Use a read-only import mount and a separate administrator configuration mount. The package store is fixed at `<data_directory>/packages`.

The worker starts only after verifying the policy pin/format/dependency inputs and obtaining exclusive store ownership. An incorrect policy pin rejects service startup. The default qualification authority remains unconnected; enabling this option does not start robots/PLCs/native processes. While P owns this store, the offline `rx-package-store` cannot open the same root concurrently.

The configured service policy defines only the scope of signature verification; it does not prove any cell's actual equipment state or process quality. Test-only policies and simulated evidence are not automatically registered as production trust.

## Incomplete conditions for proceeding to review approval

The process review/software approval API following intake is connected in [PROCESS_REVIEW.md](PROCESS_REVIEW.md). Approval is not based on an intake receipt ID alone: actual S verification material and P's independent comparison result are pinned to a specific review revision/digest. Review screens, Device/UI and procedure validation, and activation remain outstanding. The following are the full connection conditions; some have been implemented.

1. Bind fresh content verification of the same object and current policy/configuration/dependency/asset context to the independent review result.
2. For Process, check the actual S `compile_verified` result/compiler identity/resolved digest and its agreement with Cell StepBinding/site context. Device/UI require semantic verifiers for their respective kinds.
3. Pin the manifest/source/bindings/change impact and test results shown to the reviewer to an immutable review revision. Do not compress not-performed/expired/failed into PASS.
4. Check the current Verifier role, explicit target review revision, and author/reviewer separation policy. Record approval and activation as separate transitions.
5. Activation applies immutable references during operation, change-impact closure, cleanup/recovery/qualification/current conditions, and the existing base/cell authorization rules.

Current tests include failure before DB commit/lost response after commit, concurrent ID conflicts, role/terminal revocation, policy disablement/replacement, ticket expiry, rejection of proofs from other Stores/policies, historical retrieval/restart, and actual terminal mTLS startup paths. Sustained large acquisitions, disk failures, trust hot-updates during deployment and site acceptance have not been verified.
