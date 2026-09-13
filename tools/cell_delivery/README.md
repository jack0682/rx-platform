# Cell delivery-path test using the two product images

`tools/test_cell_delivery.py` uses actual P/S runtime images to check the path from initial installation to completion displayed in the operator view. The device is `FILE_SIMULATION` with an independent file ledger. Physical device addresses and device mounts are not accepted.

## Execution and inputs

Run from the P repository using Python with Playwright installed.

```sh
CARGO_INCREMENTAL=0 ../.tools/browser-python/bin/python tools/test_cell_delivery.py \
  --evidence-dir ../references/implementation/cell-delivery-new \
  --platform-image rx-platform:runtime-draft \
  --solutions-image rx-solutions:runtime-draft \
  --release-evidence ../references/implementation/phase76_checks.json
```

The evidence directory must be a new, nonexistent path. `--release-evidence` must include the exact digest of the selected S image, sealed recovery tests, and source archive. If a new image was built, create and specify new evidence from that source and its tests. Earlier image records are not validation of a new image. `--prepare-only` checks only material generation, signing, and compilation; it makes no runtime acceptance claim.

Default `--composition independent` uses three operational containers: P, Host, and Executor. `--composition supervisor` uses one P and one S container; S's product supervisor directly manages status, Host, and Executor processes. One-shot containers for preparation, inspection, and initialization are separate from the operational container count. Both compositions use two product image kinds.

Required tools are Docker, the repository Rust toolchain/cache, and Python Playwright/browser. This tool does not modify the user's ordinary ROS environment or device settings. Temporary TLS identities and signing fixtures are newly created for each run and removed at the end.

## Actual path and assertions

1. The Rust fixture exporter creates an empty initial process, public policies, and test identities. It does not inject Engine state, sessions, Host ACKs, or operating qualification.
2. Package tools in the S image assemble source material, obtain an external test signature, seal, and compile. Final installation materials bind to those actual results.
3. Start the selected composition. In supervisor mode, actual product CLIs compare file pins and installation identities, perform explicit init, and start status → Host → Executor. READY describes software process state and creates no separate operating authorization.
4. Using mTLS from a registered terminal and distinct user sessions, perform intake, review, independent approval, configuration change, confirmation of Host application, revalidation, qualification acceptance, and activation.
5. The simulated cell's six validation areas include actually measured/checked assertions and limitations. The recovery area references sealed fault tests for the exact S release. This does not mean this run itself performs power-loss tests.
6. Prepare and start a quantity-2 run in the actual operator view. Intentionally lose the start response, then recover the original request's result.
7. Compare P's material attempts, operation outcomes, and resource handovers with the Host's separate file-action records. The two operation IDs must match, with no duplicated actions. The view must also show current completion state.
8. A separate PHYSICAL cell must remain `NOT_COMMISSIONED` and reject an actual start. An operator's configuration-intake request must also be rejected.
9. Shut down Executor and Host cooperatively. In supervisor mode, send TERM to the manager once and compare the Executor → Host → status shutdown order. Each guarded child must have both actual exit 0 and STOPPED status with matching instance/PID/scope/original digest. Exceeding the observation deadline leaves a failed test; do not force termination to obtain a normal-shutdown verdict. Preserve the Host's actual safe-to-drop result too. P may exit with 2 because of unconfigured-cell rejection tests or residual Host Fence attention; do not label that a successful shutdown of the whole site.

SIMULATION permit TTL is explicitly 1 second in the public envelope/profile, separate from the 100ms inspection snapshot lifetime. This is the test configuration for this file device and delivery path, not a recommended physical timing limit. Actual runtime gates and Linux BOOTTIME are used unchanged.

## Evidence and preservation scope

`preparation.json` covers material preparation only; PASS in `result.json` covers only the runtime path specified above. Preserve public API request/response materials, tamper-check digests, independent native effects, shutdown state, and desktop/mobile screens. `public-materials` preserves explicitly selected public seeds, sealed packages, compiler outputs, signed process/revalidation reports and all referenced artifacts, and public verification policies as original bytes with per-file inventories. Login passwords, session cookies, TLS private keys, and signing seeds are not stored in evidence.

Host/Executor networks are isolated. Only P terminal HTTPS is exposed on loopback and connected to a separate terminal bridge. Python terminal connections validate the generated CA and leaf certificate. The test browser skips registering the temporary CA with OS trust but uses a separate terminal client certificate.

Cleanup is limited to simulation containers, networks, and volumes owned by this run. Shutdown performed for cleanup after failure is not counted as product normal-shutdown evidence. Actual device support, site commissioning, unattended operating reliability, and all restoration scenarios are outside this PASS scope. Supervisor-mode PASS is likewise limited to this file-device cell and the startup/shutdown paths inspected.
