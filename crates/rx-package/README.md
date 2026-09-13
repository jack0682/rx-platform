# RX package content and trust verification

The shared package envelope and content trust boundary for device, process, and UI packages. This Rust library has no ROS, device SDK, or P business-engine dependency. The same implementation is exported to S as part of the SDK.

`VerifiedPackage` means **immutable bytes that passed signature, content, target, and dependency verification**. It does not establish code quality, device behavior, site qualification, or native execution authority. Verification functions do not execute processes or scripts.

## Structure

- `manifest.json`: schema/package/version/publisher, two contract hashes and package ABI, target, entry, permissions, dependencies, assets, files.
- `manifest.sig.json`: signer key ID and Ed25519 detached signature.
- All remaining files are registered in the inventory with exact path/SHA-256/size/executable semantics. Large ONNX models, maps, and similar materials may be connected through independently verified ArtifactRef dependencies.

The signature binds the `RX-PACKAGE-MANIFEST-v1` domain, key ID, and canonical manifest bytes. Renaming a key cannot exploit another permission registration for the same public key. The package digest is the SHA-256 of the canonical manifest, distinguishing signature replacement from content identity.

JCS is used, and semantically set-like files/targets/permissions/dependencies/assets/profile lists have normalized ordering. Duplicates, case aliases, self-dependencies, and ambiguous entry roles are rejected. Versions use SemVer; build metadata cannot repackage the same version as something different.

A target with ros_distribution=None does not require a ROS runtime. It may still be used in an image with ROS installed. Packages requiring ROS specify a distribution and verify the match. ROS-independent packages are not unnecessarily coupled to a ROS version.

Ed25519 verification uses `verify_strict` from [ed25519-dalek](https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/). Use of this library and its own tests do not replace an external security audit or delivery security acceptance.

## Permissions and dependencies

| Request | Allowed package kinds |
|---|---|
| ArtifactRead | Device/Process/UI |
| ObservationRead(schema) | Device/Process |
| OperationSubmit(operation) | Process |
| NativeEndpoint(role) | Device |
| UiPanelRead(topic) | UI |

A request must satisfy both kind-specific rules and the signer's allowed set. This check is not an OS sandbox/broker that actually grants execution authority. Actual device/address bindings for endpoint roles, site approval, and Host grants/permits are separate. UI/process requests are not promoted into native access authority.

Dependencies are pinned by name, exact version, kind, and manifest digest. The transitive closure also rejects different versions of the same name, reintroduction of the root, and cycles. Existing VerifiedPackage objects are rechecked against current target/contracts and signer trust. Prior verification is not a reason to trust a revoked key. Current graph limits are depth 32/1,024 total packages.

The asset catalog must be supplied by a trusted composition boundary. Arbitrary API users must not supply policy, trust keys, or asset verification results.

## File acquisition

`directory::verify_directory` reads through a designated directory capability from [cap-std](https://docs.rs/cap-std/latest/cap_std/fs/struct.Dir.html). Parent traversal, absolute paths, Windows reserved names, backslashes, and case/hierarchy conflicts are forbidden. It does not follow symlinks and accepts only regular files. Unix opens are nonblocking to avoid indefinite open calls after FIFO replacement.

File/directory counts, depth, and total bytes are bounded. After verification, owned bytes are used rather than source paths. Later changes to source files do not change VerifiedPackage. Packages are held in memory; streaming/storage for large assets requires separate implementation.

## Storage and local verification policy

Exclusive storage of verified bytes and revalidation under current policy are documented in [STORE.md](STORE.md). P image offline import tools and S signing tools use the same policy loader. The storage result is CONTENT_VERIFIED_NOT_ADMITTED, not approval or activation.

Single-ABI policy retains `rx.package-verification-policy.v1`. To import device reference packages with ABI v2 and process packages with ABI v1 into the same store, use `rx.package-verification-policy.v2` with explicit `additional_package_abis`. The default ABI is `contracts.package_abi`; the additional list contains 1–8 entries and permits neither duplicates nor repetition of the default ABI. A v1 document does not permit additional ABIs, and a v2 document does not permit an empty list.

For example, a policy whose default ABI is `rx.package-abi.v2` may specify `additional_package_abis: ["rx.package-abi.v1"]`. This list broadens only the allowed ABI compatibility range. The two contract hashes, manifest kind/schema, target, signer kinds/permissions, and content/signature checks still apply. A policy without additional ABIs retains its existing fingerprint; changes to the additional list change the fingerprint. Receiving different packages no longer requires replacing operational policy each time, but changes to the policy itself remain subject to existing currentness checks.

## Incomplete boundaries

- Semantic validation and execution integration for actual DeviceFamily/Profile, ProcessSource, and UI descriptors.
- Production trust-key provisioning/revocation, signing services, and independent verification reports.
- Per-user/cell stage intake connects through the [intake API](../rx-application/PACKAGE_INTAKE.md). Signed process review materials and software approval also connect through the [review API](../rx-application/PROCESS_REVIEW.md). Device/UI/procedure validation, install/activate, site permission approval, and OS process sandbox/FD broker remain incomplete.
- Mixed image/package deployment, change impact, qualification, and update/restore.

Current fields/verification are a draft of the new package envelope and do not change frozen operation/cell contract files. The presence of an entry file does not establish an executable process or validated device.

Deterministic process-package assembly and external detached-signature tools connect through [S process-package](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-process-package/README.md). General production trust provisioning/activation remains separate.
