# Contributing

RX is a personal project that enables heterogeneous robots and infrastructure to cooperate through shared task, authority, state, result and recovery contracts. Repository visibility and physical validation are separate matters. Consult each repository's README and implementation records for the current validation scope.

## Branches and GitFlow

| Branch | Role | PR target |
|---|---|---|
| `main` | Stable baseline, release history and default branch | Changes must go through a PR |
| `develop` | Integration and validation of upcoming changes | Promote to `main` when ready |
| `feature/*`, `fix/*`, `docs/*`, `chore/*`, `codex/*` | Work branches created from `develop` | `develop` |
| `release/*` | Release preparation branched from `develop` | `main`; also carry required fixes into `develop` |
| `hotfix/*` | Urgent fixes branched from `main` | `main`; also merge into `develop` |
| `dependabot/*` | Automated dependency updates in this repository | Routine updates target `develop`; security updates may target `main` |

A small personal project may use a `develop` → `main` promotion PR without a separate release branch. After promotion, use a `main` → `develop` PR to bring changes and history back together. Use **merge commits** for releases, hotfixes and merges between long-lived branches to preserve their common ancestry. Work-branch PRs may also use squash merges. Do not use rebase merges.

```sh
git switch develop
git pull --ff-only origin develop
git switch -c feature/your-change
# Make the change and run the checks below.
git add <changed-paths>
git commit -m "Describe the behavior change"
git push -u origin feature/your-change
# Open a pull request targeting develop on GitHub.
```

Deleting or force-pushing `main` and `develop` is prohibited. Merges require a PR, successful CI against an up-to-date base branch and resolution of all review conversations. The required number of approvals from another person is zero so a sole maintainer can handle their own PRs. CODEOWNERS identifies review responsibility. Maintainers and automation must inspect validation results before merging. The required ruleset check is named `CI` in all three repositories.

The PR route check reads GitHub's event JSON directly. Names such as `main`, `develop`, `release/*`, `hotfix/*` or `dependabot/*` on an external fork do not grant access to this repository's release routes. Submit external contributions from work branches to `develop`. CI does not provide repository secrets or write permissions to public fork code.

## Changes and validation

Describe the problem, resulting behavior, checks actually run and unverified scope in the PR. Keep unrelated large cleanups separate from feature work. When changing authority, unknown outcomes, stopping, resource handover or recovery semantics, explain the counterexamples and compatibility impact.

The Rust toolchain in `rust-toolchain.toml` and Python 3 are required.

```sh
python3 .github/test_repository.py
python3 tools/check_repository.py
python3 tools/check_contract_baselines.py
python3 tools/test_export_host_sdk.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

These checks cover the full Rust workspace and SDK export regressions. Cross-repository integration tests marked `#[ignore]`, container tests and physical equipment tests are outside automatic core CI; provide separate execution evidence when a change affects those boundaries.

CI runs for every PR and pushes to `main`, `develop`, work, release and hotfix branches. It can also be started manually from GitHub Actions. Failed or canceled child checks cannot produce an overall `CI` success. `tools/check_repository.py` checks JSON syntax, local Markdown file links, hashes for eight normative documents and hashes for the two manifests across Git-tracked and non-ignored new files. It does not fetch external URLs, validate Markdown anchors or check the existence of sibling repository files.

## Published content

Use English for source comments, user-facing messages, documentation and contribution templates in this repository. Tests may retain intentional non-ASCII data through explicit Unicode escapes when required to preserve coverage.

Keep AI assistant instructions, prompts and local state out of Git. The ignore rules preserve local tooling files, and repository checks reject publishable assistant artifacts. Ordinary test harnesses and product source remain part of the repository.

## Changes across repositories

Original designs and contracts live in [rx_docs](https://github.com/jack0682/rx_docs), the platform and contract copies in [rx-platform](https://github.com/jack0682/rx-platform), and Host/device/services with the pinned SDK in [rx-solutions](https://github.com/jack0682/rx-solutions). Record the original contract revision, compatibility impact and manifests first, then synchronize platform specs and the solutions SDK. Link the PRs across the three repositories and record compatible commit combinations.

The platform command `python3 tools/check_host_sdk.py ../rx-solutions/sdk` checks agreement with the current platform source. Standalone solutions CI checks its SDK's own inventory; it does not prove compatibility with the latest platform. Run the cross-repository synchronization check when both repositories change. Regenerate SDK copies from platform sources instead of editing them directly.

## License and security

Contributions use the [Apache License 2.0](LICENSE). Submit only material you have the right to contribute, and preserve licenses and notices for third-party code, documents and assets. [NOTICE](NOTICE) contains RX notices and does not replace notices for external dependencies. Do not include credentials, equipment addresses or personal information in public PRs or issues. Follow the [security policy](SECURITY.md) when reporting vulnerabilities.
