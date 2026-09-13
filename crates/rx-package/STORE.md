# Storage and revalidation of verified packages

2026-09-12. Implementations: `rx-package::store`, `rx-package::policy`, and `rx-package-store` in the P image.

This phase provides the foundation for importing signed packages into RX-owned storage. The result is `CONTENT_VERIFIED_NOT_ADMITTED`. Instead of continuing to reference the source directory as execution input, bytes owned at verification time are stored separately. Successful storage alone creates no cell configuration, Run, qualification, Host grant, or operation permit.

## Identifiers and responsibilities

`ObjectId` consists of two SHA-256 values.

| Field | Hash input | Purpose |
|---|---|---|
| manifest | Manifest bytes with normalized set ordering | Identity of package content and declarations. The manifest includes the digest/size of every content file. |
| signature | Canonical signature envelope bytes | Identity of the signer key ID and detached signature. Distinguishes re-signing the same content. |

The existing `manifest_digest` semantics are unchanged. Signing the same content with a different key preserves manifest identity but creates a different storage object. Directory names under the root are `<manifest>-<signature>`. The name itself is not trusted; both digests are checked again on read.

`Store::put` accepts only `VerifiedPackage` values that passed the public verifier. It accepts neither file paths nor Boolean PASS values. `Store::verify` reacquires and verifies stored bytes under the **current VerificationPolicy** supplied by the caller through a trusted configuration boundary. Prior success or storage status does not establish current publisher/permission/dependency/asset validity.

The common P/S library checks package envelopes, signatures, file inventories, contracts/targets, dependencies, and asset references. Process semantics checking and recompilation of content belong to S's `compile_verified`. Successful process compilation also does not replace site binding, physical validation, or operating authorization.

## Storage procedure

1. Specify an administrator-owned absolute root and parent path. The root must be a real directory; a symlink at the final path is rejected. Administration of parent paths and mount configuration are trust assumptions.
2. Retain the root directory handle and acquire the exclusive `store.lock`. Later path renaming does not redirect writes into another directory. This lock provides ownership among cooperating processes; it is not a security boundary against malicious lock deletion by the same OS account or tampering by a disk administrator.
3. A new store has a `store.json` kind marker. An uninitialized directory containing other files is not adopted as a store. Write and sync the marker in a temporary file, then publish it with a no-replace rename. `open_existing` creates neither a new root nor a lock.
4. Write all manifest, signature, and content files under `.incoming-<uuid>`. Create only regular files, without execute permissions. The manifest's executable flag is preserved but is not applied as actual installation permission.
5. Sync each file, subdirectory, and staging directory. Publish the final name with a no-replace rename on the same filesystem, then sync the root.
6. Reimporting the same object reacquires every stored byte and compares it exactly. If identical, sync the root and return the same ID. Otherwise return an error. Existing content is neither overwritten nor automatically repaired.

Partially written temporary files/directories are not completed objects. `.incoming-*` remnants after interruption are neither automatically adopted nor deleted. `store.pending` left during marker initialization is not automatically recovered either. Explicit inspection/cleanup policies remain future maintenance features. If the response is lost after final publication, reimporting the same signed package can retrieve the same ID after content comparison and root sync.

This procedure relies on OS file synchronization and rename guarantees. Current tests are functional tests of process reruns, interruption remnants, corruption, ownership, and path changes. They do not validate durability against power loss, filesystem/SSD failure, or false flush reports. The first supported implementation targets Linux/macOS. A Windows publication backend is not implemented.

## Acquiring current policy

The local policy loader formerly in S was moved to common `rx-package::policy`. S's `trust` reexports it. The schema remains `rx.package-verification-policy.v1`.

- Read key IDs/publishers/allowed kinds and permissions, base/cell hashes/package ABI, and target.
- Reject duplicate key IDs. Keys removed from current policy are not restored from storage.
- Reacquire dependency-directory files, verify exact manifest digests and signatures, and validate the closure in order. Reject cycles, unresolved dependencies, and duplicate names.
- Reread actual asset bytes to verify SHA-256/size. There is no feature that copies asset bytes into this store. Later use still requires separate immutable acquisition.
- Open files with no-follow/nonblocking, then check regular-file status and size. JSON input is limited to 1 MiB, with 128 keys, 128 dependencies, and 1024 assets/256 MiB total. Each dependency/package has current loader limits of 4 MiB content and 32 files, with a separate 2 MiB metadata acquisition allowance.
- Source directory traversal is limited to depth 24, the content-file count plus 2 metadata files, and total bytes. Directory counts are limited to at most 25 entries per file. This avoids making valid deep paths unreadable after storage because of a small file inventory.

Verification uses the policy snapshot acquired in one call. Real-time revocation of trust revisions during execution and atomicity with P authority admission are not yet connected. The policy digest shown in results is the SHA-256 of **the exact policy file bytes**, not a trust revision registered in the ledger.

## Platform offline tool

The P image includes `/usr/local/bin/rx-package-store`. Its default entrypoint remains `rx-platformd`. The tool must be invoked explicitly and is not an HTTP API. It requires no ROS, S compiler, network, or device permissions. Keep the root filesystem read-only, import/configuration mounts read-only, and only the dedicated store volume writable. Manage the root and parent mounts exclusively for the tool's account.

```json
{
  "schema": "rx.package-store-config.v1",
  "store_root": "/var/lib/rx/packages",
  "import_root": "/var/lib/rx-import",
  "policy_file": "/etc/rx/package-policy.json",
  "policy_digest": "<actual SHA-256 of policy file bytes>"
}
```

The digest string above is a placeholder, not valid configuration. An administrator must supply the actual policy and its exact hash. Whitespace changes also change the pin. The CLI requires absolute configuration/store/import/policy and dependency/asset paths. Only the import target is relative, in `PackagePath` format; symlinks in every component are rejected. A package cannot specify its own trust policy or an arbitrary output path.

```text
rx-package-store import /etc/rx/package-store.json tending/package-v1
rx-package-store verify /etc/rx/package-store.json <manifest SHA-256> <signature SHA-256>
```

Import verifies the policy pin and all inputs before opening the store and storing bytes, then rereads from storage, verifies, and prints the result. Verification failure does not create a new store. Verify opens an existing store and performs the same verification. Successful stdout includes object/package/version/kind/policy digest/contracts/target and `content_semantics_verified=false`, `activation_authorized=false`. Failure exits nonzero. An error or lost stdout alone does not establish that nothing was recorded by import.

Permission to execute the offline tool is an OS account boundary. It does not replace Engineer/Verifier role checks for site accounts or user audit records. It cannot own the same store concurrently with the operational writer. The current tool also does not provide production trust installation/approval.

## P business ledger integration and subsequent order

| Stage | Inputs/checks to add | Storage/atomicity boundary |
|---|---|---|
| Intake acceptance | Current user/terminal, allowed cells, request key/body, object/signature/current trust revision | Verify/store bytes off-writer, then have the writer recheck current authority/trust revision/configuration revision and store intake record and response in one transaction |
| Review materials | Immutable source/binding, compiler/version, resolved digest, dependencies/assets/device/site context, evidence per check | Bind independent worker results to the target object and specific review revision. Distinguish negative/not performed/expired |
| Review approval | Current reviewer role, exact content/site context to approve, conflicts/authority changes | Recheck currentness on the server. Content changes after approval require a new review |
| Activation preparation | Live usage references, impact closure, cleanup/change requirements, restoration material | A change procedure that does not overwrite immutable references in current Runs. Not implemented as a simple pointer replacement |
| Activation/startup | Current qualification, mode, blocks, observations, Host generation, authorization under existing contracts | Apply start/change/authorization rules from the already frozen base/cell contracts |

Filesystem publication and SQLite commit are not one transaction. First complete the object; then the writer commits an intake record referencing its ID. An interruption between them must be able to leave only an **unreferenced object**. The reverse order is forbidden because it can leave durable intake without actual bytes. Ledger rollback or backup restoration is not grounds for erasing current physical state or existing UNKNOWN outcomes.

The intake row connects through the [per-user/cell intake API](../rx-application/PACKAGE_INTAKE.md). It uses a registered Store owner and actual verification policy fingerprint, checking current authority/context before and after worker execution. Process review materials and software approval connect through [PROCESS_REVIEW](../rx-application/PROCESS_REVIEW.md). Device/UI review, complete site review, and activation in the table remain subsequent integration prerequisites. The application SQLite schema and frozen normative files are unchanged.
