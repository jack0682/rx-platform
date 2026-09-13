# Intake, storage and reads of device action declarations

phase64. Connect common action declarations in signed DEVICE_REFERENCE packages to existing package intake. Preserve and query declarations with their original package objects without requiring people to re-enter their contents. This stage is not approval of a manufacturer-specific device verification report or application of a cell configuration.

## Common declarations and manufacturer responsibility

The shared `rx-process-contract::device_catalog::Catalog` contains installation/cell/environment/target/profile digests, required condition IDs, per-action Intents, optional native outcome tables and source document references. Limits are 64 actions, 32 conditions and 128KiB for the complete declaration. Required document roles are family/profile/adapter/operations, plus outcomes when an outcome table exists. Each document is linked by its package path and ArtifactRef.

S's JTC authoring tool generates `device-catalog.json` while producing profile/operations/outcomes from the existing assembly. The Host's JTC decoder recomputes this declaration from the same assembly and compares it. P does not import ROS or manufacturer profile structures; it checks only common-declaration correlations and source bytes.

When a verified package contains a declaration file, P checks:

- The DEVICE_REFERENCE entry's family/adapter/profile paths match the declaration.
- Every document path is a valid PackagePath, matches the actual signed payload hash/size and is not executable.
- The action list and outcome table match their referenced source documents, and every Intent's target/profile/rule matches the declaration.
- The current installation ID, intake cell and SIMULATION/PHYSICAL environment match.

These common checks do not validate manufacturer profile semantics, calibration suitability, control authority or physical behavior. The API always returns `manufacturer_validation_required=true`, `activation_authorized=false`. No endpoints were added to install declarations as actual StepBindings or create qualification/Arm/Run.

## Durability and currentness

File acquisition, signature checks and asset verification run outside the writer in the existing bounded package worker. `Prepared` binds an actual registered Store's immutable object to a ticket and cannot be deserialized from the wire. Passing signature checks does not produce Prepared if the declaration contradicts its source.

The authoritative commit rechecks current identity, role, cell, installation, policy registration, configuration digest and ticket expiry. The canonicalized declaration, its ArtifactRef, receipt/original object ID/event/request-key result are stored in the same transaction. A pre-commit failure leaves no receipt/declaration; the same key after a lost response returns the original receipt. An acquired object may remain in the file Store, but that does not create intake approval or execution authority.

The receipt's optional `device_catalog` references the canonicalized projection. The original signed payload is preserved in the package Store referenced by receipt.object. Reads serialize/hash/size-check the stored declaration again and verify receipt/installation/cell correlations. New settings do not overwrite historical declarations.

`review_context_current` means only that the current cell configuration and package service registration match those at intake. It can become false after restart/policy re-registration/configuration change. Because reads do not recheck the source Store, `content_reverification_required=true` is retained. Historical reads also require current login/cell access and an Engineer/Verifier role.

## API and UI

```text
GET /api/v1/package-intake/device-catalog?cell=CELL&id=INTAKE_ID
```

Returns cell/intake/object/reference/catalog and the currentness/revalidation-required flags above. Historical packages without a catalog return reference/catalog=null. Existing receipts remain readable without the new optional field. The eight common wire normative files and six optional bindings are unchanged.

The operator app's package review page recognizes DEVICE_REFERENCE format. Selecting a device displays target/environment/conditions/action resources and timing/outcome mappings/source references, and allows the data to be downloaded. Device packages do not display a process review request button. If the response's cell/intake/object/reference differs from the selected record, it is not accepted as current content. Late responses follow existing generation/abort controls; expired/error/policy-mismatched data is labeled as archived material.

## Validation and follow-up

Tests cover source/mapping mismatches, path/hash tampering, installation/environment/cell/ticket expiry, atomic commit/lost responses, authority and historical receipts. Browser integration imports a package created by the actual S JTC CLI and an external test signer through the actual P Store/API, then verifies that the downloaded declaration matches the signed package's declaration. It also checks that cell configuration digest/operating state remain unchanged. Exact final results are in the [phase64 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase64_checks.json).

The next stage binds manufacturer-validator verification evidence and independent review to the device package object/declaration digest, and connects approved actions to cell configuration change plans. This screen currently reads declarations and is not counted as a completed device-approval screen. Production JTC Authority/lifecycle/fencing and physical calibration/support/acceptance remain incomplete; the first physical cell is NOT_COMMISSIONED.

phase65 added the [device software report/independent approval API](DEVICE_REVIEW.md). A separate authority and reports from the actual S decoder are compared against current sources. This declaration-read response does not aggregate that approval state; approval UI, actual configuration application and physical qualification remain follow-up work.
