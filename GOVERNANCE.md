# Repository governance

`repository-settings.json` is the reviewed source for GitHub settings in this repository. The rules below apply to the three RX repositories. Local hooks make mistakes visible before a push; GitHub rules and required checks enforce the remote boundary.

## Protection layers

| Layer | Scope | Requirement |
|---|---|---|
| Signed contributions | Every branch | GitHub-verified commit signatures |
| GitFlow branch names | New branches outside the allowed names | Creation is rejected, with no bypass actors |
| Protected branches | `main`, `develop` | PR, current-base `CI`, `DCO`, verified signatures, resolved review threads, no deletion or force push |
| PR-only updates | `main`, `develop` | Updates are restricted; the administrator's exception applies only through a PR |
| Immutable tags | Every tag, including archival tags | No update or deletion, with no bypass actors |
| Local hooks and commit CI | Every contribution, including merges | Author-matching DCO trailer and verified OpenPGP commit signature |

`CI` must come from GitHub Actions (app 15368), and `DCO` from the DCO app (app 1861). Push builds report `Branch CI`; they do not substitute for the PR check. The protected-branch quality rules have no bypass actors. The separate PR-only update exception cannot skip CI, DCO, signatures or review-thread resolution.

Commit-policy CI checks that each signoff matches the author and that the verified signature is OpenPGP. The DCO app also checks contributions with owner exemption and remediation commits disabled. Its default handling of merge and bot commits does not weaken the repository's own commit audit.

Native commit-message and branch-name metadata rules are an Enterprise organization feature; GitHub rejected them for these personal repositories. A creation restriction instead rejects branch names outside `main`, `develop` and the documented work prefixes. DCO is enforced by required PR checks and local hooks; the PR route check also validates how those branches may merge. A signed work-branch push can reach GitHub before those checks complete, but it cannot merge into `main` or `develop` until the required checks pass. The merge helper adds and verifies the final merge commit's DCO trailer; GitHub's native signed-commit rule alone cannot require that trailer. Use the helper for every merge.

Squash merge, rebase merge and automatic merge are disabled. Use the merge helper so the GitHub-generated merge receives the authenticated merger's signoff. GitHub signs that merge with its web-flow OpenPGP key; individual commits use their contributors' registered keys. The update-branch shortcut is disabled; update a work branch locally with a signed merge.

Zero external approvals are required while there is one maintainer, because a person cannot approve their own PR. All review conversations must still be resolved. Once another maintainer can review, change the approval count and required code-owner review through a governance PR.

Repository owners can still edit repository administration settings. These controls prevent ordinary direct pushes; they cannot remove the owner's GitHub administrative authority. Keep administrative credentials separate from daily development credentials when possible. No CI job receives an administrative token or private signing key.

## Configure signing once per clone

Install Git, Python 3, GnuPG and GitHub CLI. Use an email verified on your GitHub account, and add your **public** GPG key to [GitHub's signing-key settings](https://github.com/settings/keys). Keep the private key on your trusted signing device.

```sh
git config --local user.name "Your Name"
git config --local user.email "your-verified-email@example.com"
git config --local user.signingkey YOUR_FULL_GPG_FINGERPRINT
python3 tools/install_git_hooks.py
```

The installer configures this clone's hooks and OpenPGP signing defaults. The commit hooks add the current committer's signoff and reject a missing author certification. They do not add another person's certification. The pre-push hook refuses updates or deletion of `main` and `develop` and checks outgoing commit and tag signatures.

Use `git commit -s -S` explicitly even with the hooks installed. If GPG asks for a passphrase, unlock the key through your normal GPG agent. Do not put passphrases or secret keys in repository files or Actions secrets.

## Develop, integrate and release

1. Branch from current `develop` using a prefix listed in [CONTRIBUTING.md](CONTRIBUTING.md).
2. Make focused, signed and signed-off commits. Explain behavior and executed validation in the PR.
3. Push the work branch and open a PR targeting `develop`.
4. Review the diff, wait for `CI` and `DCO`, resolve review conversations, then use the merge helper.
5. Promote validated `develop` to `main` with a PR, or prepare a `release/*` branch first.
6. Backmerge `main` to `develop` through a PR after promotion. An urgent `hotfix/*` starts from `main` and must also reach `develop`.

```sh
python3 tools/merge_pr.py PR_NUMBER
```

The helper binds the merge to the inspected PR head and required check providers. It adds a signoff for the authenticated merger, requests a merge commit and verifies the resulting commit. A changed head or failing requirement stops the merge. It does not bypass rules.

To bring a work branch up to date, fetch the target and create a signed merge. Inspect conflicts and rerun the affected checks before pushing.

```sh
git fetch origin
git switch feature/your-change
git merge -S --signoff origin/develop
git push
```

For a release, inspect the successful `main` build, prepare release notes with the exercised validation scope, then create a signed annotated tag at the intended `main` commit. Choose a new version; published tags are immutable.

```sh
git fetch origin
git tag -s v0.2.0 origin/main -m "RX 0.2.0"
git verify-tag v0.2.0
git push origin refs/tags/v0.2.0
```

The version above is an example, not a release instruction. GitHub's native signature rule covers commits; local hooks and release review verify the tag object's OpenPGP signature. A GitHub release should reference the already verified tag. Routine source and tag history is never rewritten after publication.

## Fix a rejected contribution

For your own latest unpublished commit, add the missing signoff and regenerate the signature:

```sh
git commit --amend --no-edit -s -S
```

For several of your own commits, coordinate before rewriting a shared work branch, preserve a local backup and use a signed rebase with signoffs. Keep the original author identity. Do not certify someone else's contribution on their behalf. Push a rewritten work branch with an explicit `--force-with-lease`; never rewrite `main`, `develop` or published tags as a routine repair.

Bot proposals are subject to the same commit audit. If an automated change cannot produce the required author certification, review and reproduce the dependency update as your own contribution with truthful attribution and preserved third-party notices. Do not create a bot exemption or copy a human signoff onto a bot-authored commit.

If CI or the DCO app is unavailable, leave the PR open and rerun or restore the failing service. A green check from another provider, a previous revision, an override button or a local hook bypass is not a replacement for current evidence.

## Audit and change the policy

```sh
python3 .github/test_repository.py
python3 .github/test_commit_policy.py
python3 tools/check_repository.py
python3 tools/check_commit_policy.py --all --github-repository jack0682/rx-platform
python3 tools/configure_github.py
```

The commit audit above uses GitHub's verification results with the logged-in GitHub CLI account. Without `--github-repository`, verification uses your local GPG keyring; import trusted contributors' public keys, including GitHub's web-flow public key, before auditing their signatures. An unknown local public key alone does not establish that a GitHub-verified signature is invalid.

The GitHub configuration command is read-only by default. It compares repository settings, Actions permissions, security settings, automatic security fixes, vulnerability-alert availability and the complete managed rulesets with the reviewed file. To change policy, submit a PR that changes the settings and explains the effect, merge it under the existing rules, then run `python3 tools/configure_github.py --apply` with an administrator account. Re-run the read-only audit and inspect the effective branch rules. No automated job silently edits protections.

Actions use pinned action revisions, read-only tokens and no `pull_request_target` execution of contributor code. Secret scanning and push protection remain enabled. Dependency updates use manually reviewed, signed contributor PRs. Vulnerability notifications remain enabled; automatic security-fix PRs are disabled.

The [2026-09-14 signing migration](docs/governance/history-rewrite-2026-09-14.md) records the one-time user-authorized history rewrite, unchanged file trees, old-to-new commit correspondence and archived backup digests. Old closed PR checks are historical records; the migration does not retroactively rerun or change them. Fresh clones use the rewritten branches and tags. Keep local uncommitted work before replacing an older clone.

## References

- [GitHub rules and enforcement](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets)
- [GitHub metadata-rule availability](https://docs.github.com/en/enterprise-cloud%40latest/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets#metadata-restrictions)
- [GitHub rulesets REST API](https://docs.github.com/en/rest/repos/rules)
- [DCO application configuration](https://github.com/dcoapp/app)
- [Developer Certificate of Origin](https://developercertificate.org/)

## Dependency update policy

Dependency changes are maintained through the manual, signed PR procedure in [CONTRIBUTING.md](CONTRIBUTING.md#manual-dependency-updates). There is no scheduled version-update configuration on `main` or `develop`. Vulnerability alerts remain enabled so maintainers can assess and prepare security fixes themselves.

`repository-settings.json` declares `dependency_updates.automated_security_fixes: false`. Applying the reviewed configuration disables automatic security-fix proposals and retains vulnerability alerts; the read-only audit also checks both states. Do not run an older configuration script that unconditionally enables automatic fixes. The regression tests exercise applying and reapplying this policy, detecting reactivation without changing remote state, and detecting a disable request that did not take effect.

### Closed proposals — 2026-09-15

Four earlier proposals remain closed and unmerged: [#7](https://github.com/jack0682/rx-platform/pull/7) (checkout/cache revisions) and [#9](https://github.com/jack0682/rx-platform/pull/9) (base64 0.23.1) passed their Rust jobs but failed the author DCO audit. [#8](https://github.com/jack0682/rx-platform/pull/8) (sha2 0.11.0) failed Rust with E0277 / `LowerHex` in `crates/rx-protocol/build.rs:22`. [#10](https://github.com/jack0682/rx-platform/pull/10) (argon2 0.6.0) failed with E0432 / `SaltString` and E0061 in `crates/rx-api/src/auth.rs`. Those two upgrades need source/API migration; none of the four upgrades is applied here.

The [complete ten-proposal register](https://github.com/jack0682/rx_docs/blob/eabb60fb7865018df566a36ea6344f8a01e433c2/GOVERNANCE.md#dependency-update-policy) preserves the proposed versions, three observed Rust failures, checks that passed on the other seven proposals, and historical CI links. These observations concern those historical PR revisions; re-run the relevant checks for a new maintainer-authored update. No partial CI success or successful DCO app status overrides the repository commit audit.


## Recorded historical DCO incidents

The byte-pinned [incident ledger](.github/historical-dco-incidents.json) records two unresolved author-signoff violations from 2026-09-16: `rx-solutions@041f4724cf3ab96debc9e8f08d25a64274d41ca0` and `rx_docs@3df96357f43f4d2e9e81e871aaa7f844fc004fd2`. This is a narrowly scoped policy exception, not a retroactive author certification. The original commit bytes and history remain unchanged. The audit permanently labels each encountered incident as **HISTORICAL DCO VIOLATION (unresolved)** even when its policy result succeeds.

Only the exact repository, full commit SHA and original author tuple is admitted without a signoff. Verified OpenPGP signatures remain mandatory for those commits too. No other commit, author, merge, bot, date or branch is exempt. A missing or changed ledger fails audit. Updating both the checker pin and the ledger is an explicit reviewed governance-policy change; these writable repository files are not an external tamper-proof authority.

PR and push CI both use `check_commit_policy.py --head HEAD_SHA` to audit the complete ancestry of the actual contribution head. The manual `--range` mode remains available for a deliberately limited diagnostic; it is not the CI acceptance scope. Full ancestry prevents a new violation from being hidden behind an incremental range's base. The ledger is identical in all three repositories; each entry is applied only in its named repository.

Always use `tools/merge_pr.py` for a merge. A PR-head check cannot inspect a future merge message; a new merge without its author trailer is still rejected by branch CI. This policy does not add a GitHub server-side final-message rule, bypass branch protection, rewrite main, or automatically promote develop. Local signing hooks and pre-push checks remain strict for new contributions.
