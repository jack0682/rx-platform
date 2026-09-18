# CI guard boundaries

The repository job runs all seven `tools/update_*_binding.py` scripts with `--check`. They compare the complete expected manifest, including metadata, without rewriting it. The Rust protocol build checks only the `source_sha256` entries of those manifests. In an isolated copy, changing host-read `max_payload_bytes` from 1000000 to 1000001 made its Python check fail while `cargo check -p rx-protocol --locked` still succeeded.

## Why the two base schemas are not optional bindings

Decision, 2026-09-18: retain the existing separation. Do not add `proto/rx/contract/v1/contract.proto` or `proto/rx/cell/v1/cell.proto` to an optional binding manifest.

The repository evidence supports a deliberate separation of contract identities rather than an accidentally removed inventory entry. This is an architectural inference from the sources below, not a recovered statement of the original author's intent.

- The [base manifest](../spec/contracts/v1.0/protocol_manifest.json) and [cell manifest](../spec/cell_operations/v1.0/protocol_manifest.json) identify normative document bytes. The cell manifest also pins the base manifest. Neither claims to inventory protobuf source bytes.
- The [optional executor binding](../spec/executor/v1/README.md) explicitly supplements those contracts and does not replace their mutations or identities. The [checkpoint binding](../spec/executor-plan/v1/README.md) likewise separates its identity from both frozen manifests and executor-read. Each of the seven optional bindings has its own declared sources and metadata; placing a base schema in one would assign it an unrelated owner.
- The existing [wire tests](../crates/rx-protocol/tests/wire.rs) compare generated descriptors against numbered fields in the normative documents and separately verify that document-manifest hashes remain the negotiation identities. A fresh run passed all 11 tests and compared 403 numbered source fields.
- `git log --all -- proto/rx/contract/v1/contract.proto proto/rx/cell/v1/cell.proto` traces both files to initial commit `493a6bb1a5d2e1ac7a49675ff64c6602b1082591`. Searching all fetched local refs with `git log --all -G 'contract.proto|cell.proto' -- 'spec/**/binding.json' 'tools/update_*_binding.py'` returned no addition or removal. This history supports the decision but cannot prove intent or describe unavailable history.
- Both files are byte-pinned in the exported SDK's `source-lock.json`. `check_host_sdk.py` checks that inventory and compares it with a fresh platform export. At the measured baseline, all 107 SDK files matched. The SDK pin is a copy-integrity check, not a platform source freeze.

The three mechanisms remain distinct: normative document identity, optional source bindings, and SDK inventory. There is **no platform byte freeze for these two protobuf files**. The descriptor tests are not an exhaustive proof of wire conformance: for example, their document loop skips unmatched message names. Stronger conformance or a separate base-source inventory would require its own requirement and validation, not a silent extension of an optional binding.

## Manual audit boundary

The two manual jobs in [ci.yml](../.github/workflows/ci.yml) run on `workflow_dispatch` and are absent from the required aggregate's `needs`. They compare SDK freshness and audit GitHub settings with a read-only token. Audit failure remains visible independently of the aggregate. See [the contribution workflow](../CONTRIBUTING.md#changes-across-repositories) for commands and the inactive cron boundary.
