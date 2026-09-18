# Contributing

RX is a personal project for heterogeneous robots and infrastructure. Contributions use Apache-2.0. Consult the repository README and implementation records for the current validation scope.

## Branches and GitFlow

| Branch | Purpose | PR target |
|---|---|---|
| `main` | Stable baseline and release history; default branch | PRs only |
| `develop` | Integration and validation of upcoming work | `main` when ready |
| `feature/*`, `fix/*`, `docs/*`, `chore/*`, `codex/*` | Work created from `develop` | `develop` |
| `release/*` | Release preparation created from `develop` | `main`, then carry fixes into `develop` |
| `hotfix/*` | Urgent fixes created from `main` | `main`, then carry fixes into `develop` |

All PRs use merge commits. Squash and rebase merges are disabled so that reviewed commits, signatures and signoffs retain their identities. A small release can use a `develop` to `main` promotion PR without a release branch. After promotion, merge `main` back to `develop` through a PR.

## Signed commits and the daily workflow

Every commit, including merges, needs both a matching author `Signed-off-by` trailer and a verified OpenPGP signature. Read the [Developer Certificate of Origin](https://developercertificate.org/) before signing off. The trailer records your certification of contribution rights; the cryptographic signature authenticates the commit. Neither substitutes for the other.

Configure a verified GitHub email and register your public GPG key, then install the repository's local hooks. See the [repository governance guide](GOVERNANCE.md) for key setup, branch updates, merge and recovery instructions.

```sh
python3 tools/install_git_hooks.py
git switch develop
git pull --ff-only origin develop
git switch -c feature/your-change
# Make the change and run the checks below.
git add <changed-paths>
git commit -s -S -m "Describe the behavior change"
git push -u origin feature/your-change
# Open a PR targeting develop; wait for CI and DCO.
python3 tools/merge_pr.py PR_NUMBER
```

`main` and `develop` reject direct pushes, force pushes and deletion. A PR needs `CI` from GitHub Actions and `DCO` from the DCO app, an up-to-date base, verified signatures and resolved review conversations. A separate update lock permits the administrator to update these branches only through a PR; that exception does not bypass the quality rules.

The required number of approvals is zero while there is only one maintainer. CODEOWNERS identifies review responsibility. The maintainer still reviews the diff and validation evidence before merging. When an independent maintainer joins, raise the required approvals and enable required code-owner review together.

External forks cannot use names such as `main`, `develop`, `release/*` or `hotfix/*` to acquire this repository's release routes. Submit external changes from work branches to `develop`. Fork CI receives a read-only token and no repository secrets.

## Manual dependency updates

Review vulnerability alerts regularly and after a relevant advisory. A maintainer owns dependency updates; automatic version and security-fix proposals are disabled. Evaluate release notes, compatibility, licenses and security impact before changing a manifest or lockfile.

1. Start from current `develop` with a focused `chore/deps-<package>` branch.
2. Update the required manifests, lockfiles and pinned action revisions together. Keep unrelated upgrades separate and explain any required source changes.
3. Run the repository checks and the affected language, application and integration tests listed below. A previously passing proposal is not evidence for a new revision.
4. Commit with your own truthful `Signed-off-by` and OpenPGP signature using `git commit -s -S`, then open a PR to `develop`. Wait for current `CI` and `DCO` before using `tools/merge_pr.py`.
5. For an urgent fix to the stable release, use the existing `hotfix/*` route from `main`, then synchronize `main` back to `develop` through a checked PR.

See [the closed dependency proposals and policy record](GOVERNANCE.md#dependency-update-policy) before revisiting an earlier upgrade. Do not add someone else's signoff or weaken the commit audit to reuse an automated commit.

## Changes and validation

Describe the problem, resulting behavior, checks actually run and unverified scope in the PR. Keep unrelated large cleanups separate from feature work. When changing authority, unknown outcomes, stopping, resource handover or recovery semantics, explain the counterexamples and compatibility impact.

The Rust toolchain in `rust-toolchain.toml` and Python 3 are required.

```sh
python3 .github/test_repository.py
python3 .github/test_commit_policy.py
python3 tools/check_repository.py
python3 tools/check_contract_baselines.py
python3 tools/test_export_host_sdk.py
for checker in tools/update_*_binding.py; do python3 "$checker" --check; done
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

The required repository job checks all seven optional binding manifests with `--check`, including metadata that the Rust build's source-hash check does not compare. It never regenerates those manifests.

The [CI guard boundaries](docs/ci-guards.md) explain why the two base protobuf schemas remain separate from those optional bindings and what the existing checks do not prove.

Run `gh workflow run ci.yml --ref develop` to also run the SDK freshness and read-only GitHub configuration audits. The SDK audit compares the selected platform revision with a separate checkout of solutions `develop`, and logs both revisions. These manual jobs are excluded from the required `CI` aggregate and do not run on pushes or PRs. A failed audit remains visible as a failed job; a green aggregate does not establish audit success.

No cron is active. Scheduling requires an explicit workflow change on the default branch during a future reviewed `main` promotion; promotion alone does not add a schedule. The configuration audit uses only the read-only workflow token. If GitHub denies access to administrative settings, retain that failure and run `python3 tools/configure_github.py` locally with an authorized account. Local success does not prove the workflow token has access. CI never applies server settings or receives an administrative token.

## License and security

Contributions use the [Apache License 2.0](LICENSE). Submit only material you have the right to contribute, and preserve licenses and notices for third-party code, documents and assets. [NOTICE](NOTICE) contains RX notices and does not replace notices for external dependencies. Do not include credentials, equipment addresses or personal information in public PRs or issues. Follow the [security policy](SECURITY.md) when reporting vulnerabilities.
