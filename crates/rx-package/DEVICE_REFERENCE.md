# Device implementation reference package v2

2026-09-12. An extension to package content verification. It neither changes frozen base/cell protocol manifests nor creates device operating qualification.

## Why a separate entry is needed

The existing `DEVICE` entry requires an executable adapter file in the signed inventory. Selecting an adapter statically included in an image needs a different representation. A JSON descriptor must not be marked executable to pass verification. The new entry is **`DEVICE_REFERENCE`**, using `schema=rx.package.v2` and `package_abi=rx.package-abi.v2`.

| Format | entry | Adapter material | Verification |
|---|---|---|---|
| v1 | DEVICE | Executable | Existing requirement retained |
| v1 | PROCESS / UI | Declarative content | Executables remain forbidden |
| v2 | DEVICE_REFERENCE | Non-executable implementation descriptor | All executable payloads forbidden |

Currently, v2 supports only DEVICE_REFERENCE. DEVICE_REFERENCE labeled with v1 schema/ABI is rejected. Older readers do not know the new entry and reject it rather than interpret it as an existing executable adapter. Policy must also explicitly allow that ABI. There is no automatic conversion or migration.

## Signed semantics

An entry has `family`, a nonempty list of unique `profiles`, and `adapter`. Each path points to a distinct file in the signed inventory. Publisher-kind and permission checks treat it as PackageKind::Device; it does not receive the Process OperationSubmit permission.

The common verifier checks signatures, key scope, schema/ABI/target, canonical identity, inventory/bytes/digests, dependencies/assets, and existing path boundaries. `manifest_bytes` sorts profile paths for both device formats. The existing signing domain remains; schema/ABI/entry kind are included in the signed content, so changing the kind requires a new signature.

The verifier neither loads shared libraries nor executes descriptors. It also does not select drivers or judge device semantics. A resolver pinned in the product must check the descriptor schema, exact implementation identity, profiles, and current environment, and reject unknown implementations. Package signatures establish provenance/content, not physical qualification.

## Compatibility and validation

`device_reference_v2_has_explicit_version_and_no_executable_payload` checks signed v2, incorrect schema/ABI, forbidden executable payloads, and preservation of v1's executable requirement. Existing tests cover signatures, permissions, paths, ownership of source content, and trust changes. SDK export delivers the changed model/verifier to solutions while retaining base/cell normative files.

The first resolver is [MELSEC Host startup](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/DEVICE_PACKAGE_STARTUP.md). Mixed v1/v2 dependency coordination is outside this extension. MELSEC Template/Site authoring and signing tools connect through [rx-device-package](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-device-package/README.md). The first resolver does not allow package dependencies. Execution results and source hashes are in the [phase58 validation record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase58_checks.json).
