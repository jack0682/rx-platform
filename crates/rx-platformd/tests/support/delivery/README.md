# SIMULATION acceptance-input exporter for the two actual images

All entrypoints are ignored `rx-platformd` tests and create no Runtime/Host/Executor sessions, qualification, or Runs. They do not change P production authority. An external harness runs CLIs from the actual S image, both product images, and the browser/API.

## A: Process/package inputs

Set `RX_CELL_DELIVERY_OUTPUT=<new absolute directory>` and optional `RX_CELL_DELIVERY_ARCH=arm64|amd64` (default arm64), then run `cargo test -p rx-platformd --test delivery_fixture --locked export_delivery_seed -- --ignored --exact`.

Outputs are `seed.json`, `initial-cell.json`, `compile-input.json`, `package-recipe.json`, `package-policy.json`, `assets/<sha>.bin`, `scope.json`, and `signing-fixtures.json`. Base/cell hashes are computed from canonical bytes of the actual checked-in protocol_manifest. Source is a one-action process for FILE_SIMULATION, explicitly specifying ready observations, a sim/ready guard, and rx.sim.completed.v1 code0 completion. Program/parameter and every semantic digest also have actual input bytes.

Supply S CLIs only a public seed copy, excluding private signer files, at `/config:ro`. Execute `assemble → request → external test signature → seal → compile` with actual `rx-process-package`. Final compilation must run from the **sealed package** so package_digest references the actual manifest. Preserve validator_digest from `validator-identity` stdout as well. The exporter neither fabricates nor generates ResolvedProcess or compiler verification reports.

## External test signing

Provide `sign_delivery` with `RX_CELL_DELIVERY_SEED=<A>`, `RX_CELL_SIGN_INPUT=<absolute JSON>`, and `RX_CELL_SIGN_OUTPUT=<new absolute signature JSON>`. Input uses one of the following formats.

```json
{"key":"delivery-package-signer","message_hex":"<message_hex from the actual S signing request>"}
```

```json
{"key":"delivery-qualification-signer","qualification_report":"<absolute path to qualification.json authored by the parent from actual evidence>"}
```

Allowed keys are delivery-package-signer, delivery-review-signer, and delivery-qualification-signer. Do not copy other S signing-request JSON metadata into input; forward only key/message_hex. In addition to the signature envelope, output includes a sidecar with extension `.metadata.json` (for example, `verification.sig.json` → `verification.sig.metadata.json`). In qualification mode, the sidecar contains the exact Report::digest, and signing_message is also computed by the existing q model. It issues no policy, verification, or execution authority.

Test private seeds are stored only in A's `signing-fixtures.json`. Do not copy them into P/S runtime configuration, images, or evidence storage. Public keys per key ID are bound in A's seed and policy and compared again by the signer. B does not export the CA private key.

## B: Final startup/revalidation policy

Required environment for `export_delivery_final`:

- RX_CELL_DELIVERY_OUTPUT: new final directory
- RX_CELL_DELIVERY_SEED: A directory
- RX_CELL_RESOLVED: resolved.json from actual S sealed-package compilation
- RX_CELL_PACKAGE: actual S sealed-package directory
- RX_CELL_COMPILER_ID: 64-character digest from actual S validator-identity
- RX_CELL_QUALIFICATION_VALIDATOR_ID: digest identifying the parent's actual evidence harness/verification procedure
- RX_CELL_DELIVERY_PORT: external terminal HTTPS port
- RX_CELL_OPERATOR_BUNDLE: selected actual S UI dist (optional). Supplied to P as `/operator:ro`

B checks package signatures/content, signed compile-input=A, and actual resolved source/package/action/structure. Only the local verification copy of policy asset paths is changed to A paths; public package policy bytes remain unchanged. For the one-action process, it calculates the target using the same step-ID conversion as actual P process-change. **Change.after.sha256 returned by the P API must equal target_configuration_digest in delivery.json.** If they differ, do not change live policy; restart with a new disposable installation.

Outputs:

- `config/`: P startup/catalog/credentials/public policies, P server and P → H TLS files, program/parameter assets
- `host-config/`: H startup template, shared binding inputs, Host server/publisher TLS files
- `executor-config/`: E cell template and E client TLS files
- `browser/`: registered terminal cert/key and public CA. Used only outside containers
- `import/package/`: verified actual S signed package
- `reference/`: initial/target/physical-negative cells and actual resolved output
- `qualification-materials/`: exact v2 policy and original artifact pool. Generates neither result reports nor PASS
- `delivery.json`: environment/identifiers/target/contracts/validators/subsequent patch stages
- `browser-fixture.json`: test login information, terminal paths, role-specific lists of allowed mount files

Installer has bootstrap AccountAdmin+Engineer; engineer/verifier/release/operator roles are separated. The browser terminal is registered using its actual leaf DER fingerprint; only H publisher/E fingerprints enter the P gRPC service allowlist. No Host/Executor sessions are created.

Hello.peer_id for P → H is a Name formed from the same installation UUID used by product ConnectionService. Both Host allowed_platform_certificates values and binding-input.platform use this value, also recorded as delivery.json.platform_peer. TLS client certificate SAN/CN labels do not replace this peer ID; the allowlist binds the actual leaf fingerprint.

`cell/physical-unconfigured` has a different definition/Host/Executor/resource/scope/profile/site from main and has no physical endpoint, host_link, or qualification profile. Models/addresses are not inferred. Operator/Engineer may check CreateRun/start rejection for this cell; its Prepared Run does not overlap main qualification's quiet/impact set.

## Explicit parent patches and actual execution stages

P mounts are config → /config, import → /import, and a separate data volume → /data. P data is /data/platform and runtime is /data/runtime; the parent prepares writable directories owned by UID10001. gRPC is https://p:7443, Host is https://s:7444, and terminal is https://127.0.0.1:PORT.

First use actual P init/run and terminal login/overview to read installation.store_generation. Place that value in Host publisher.store_generation and E expected_service.scope.store_generation. The parent creates Host bindings in S format from exact cell/Intent/condition values in binding-input.json and pins their sha256. Obtain the E engine pin from /opt/rx/bin/rx-bt-engine in the actual S image. PATCH strings in the templates are deliberately invalid UUIDs/digests, so execution is impossible before patching. After patching, publish role-specific files as `startup.json`/`bindings.json`/`cell.json`, then perform H/E init/run.

P configuration contains no H/E/browser/signing private keys; role-specific mount-file lists are in browser-fixture.json. Do not arbitrarily mount the entire final root at /config in one container.

Later qualification reports use the complete actual Request from P Begin and the parent harness's actual outputs for each area. Connect software/compiler, Host/equipment, Fence/cohort integration, validated recovery evidence, PHYSICAL/authority rejection, and separate-account/non-operating state through each criterion's evidence_schema. Not-performed checks are NOT_RUN; neither the existence of exporter specifications nor labels are grounds for PASS. Ledgers, states, and original records come only from APIs/product programs; test authority or direct DB seeds must not replace the first qualification.
