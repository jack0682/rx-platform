# RX Platform

> **Development has moved to [RobotTransformation](https://github.com/jack0682/RobotTransformation).**
> New product and documentation work belongs in that repository. Development here has ended.
> This repository preserves its original history, tags and existing references; the instructions below describe the legacy source tree.
> This move does not accept the preserved CP2 Run or authorize changes to its environment or physical equipment.

The [local simulation skill service](crates/rx-api/SOFTWARE_SKILLS.md) reuses RX's
application rules, state writer, storage and Operation model for authored Python
computations. Its installer and execution worker are provided by rx-solutions.

[![CI](https://github.com/jack0682/rx-platform/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jack0682/rx-platform/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)

Vendor-neutral Rust services for coordinating heterogeneous robots and infrastructure through explicit task, authority, observation, result and recovery contracts.

RX aims to support complete workflows across participants and operating domains. The current implementation is an early runtime foundation with simulated acceptance evidence; city-scale operation and physical deployment remain future validation work.

## Responsibilities

- Pure domain rules for task identity, execution knowledge, results and resource disposition.
- Durable execution authority, dispatch records, observations and recovery.
- Versioned protocol, package and process contracts.
- Operational APIs and local runtime composition.

Device integrations, workflow executors and operator applications live in [rx-solutions](https://github.com/jack0682/rx-solutions). Project direction and normative originals live in [rx_docs](https://github.com/jack0682/rx_docs).

## Build and check

Install the Rust toolchain declared in [rust-toolchain.toml](rust-toolchain.toml). The repository builds without a sibling checkout.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 tools/check_contract_baselines.py
python3 tools/test_export_host_sdk.py
```

The [local API guide](crates/rx-api/README.md) describes development startup. The local service uses the real writer and SQLite implementation; hardware launch and physical qualification are separate integration responsibilities.

## Contract and SDK ownership

Normative build inputs are pinned in `spec/`. SDK exports contain shared types and adapters while excluding application authority. When working with a sibling solutions checkout:

```sh
python3 tools/export_host_sdk.py ../rx-solutions/sdk
python3 tools/check_host_sdk.py ../rx-solutions/sdk
```

An export's own source-lock is insufficient if it differs from current platform sources.

## Repository workflow

`main` is the stable baseline; `develop` integrates changes. Feature work targets `develop`, release/hotfix work targets `main`, and release changes are merged back to `develop`. See [Contributing](CONTRIBUTING.md), [Security](SECURITY.md) and the [PR template](.github/pull_request_template.md).

Required checks, merge policy and branch/tag rules are recorded in [repository-settings.json](repository-settings.json). Audit the live configuration with `python3 tools/configure_github.py`; use `--apply` only when intentionally changing repository administration.

## License

RX project contributions are licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE). Third-party components retain their own notices and licenses.
