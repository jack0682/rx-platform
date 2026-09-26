#!/usr/bin/env python3
"""Check author DCO trailers and verified OpenPGP signatures; implement local hooks."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

PROTECTED = {"refs/heads/main", "refs/heads/develop"}
ROOT = Path(__file__).resolve().parents[1]
INCIDENTS = ROOT / ".github/historical-dco-incidents.json"
# Changing this pin is a governance-policy change, never an automatic repair.
INCIDENTS_SHA256 = "c27c932788f695355e4d8eca4a48fe6f9f6d24c16ad1e40140ff632911e54a4b"


def historical_incidents():
    data = INCIDENTS.read_bytes()
    if hashlib.sha256(data).hexdigest() != INCIDENTS_SHA256:
        raise ValueError("Historical DCO incident ledger changed; reviewed policy bytes are required")
    return json.loads(data)["incidents"]


def run(*command, input=None):
    result = subprocess.run(command, input=input, text=True, capture_output=True)
    if result.returncode:
        raise ValueError(result.stderr.strip() or f"Command failed: {command[0]}")
    return result.stdout


def git(*args, input=None):
    return run("git", *args, input=input)


def identity(kind):
    value = git("var", f"GIT_{kind}_IDENT").strip()
    match = re.fullmatch(r"(.+ <[^<>\s]+>) \d+ [+-]\d{4}", value)
    if not match:
        raise ValueError(f"Invalid Git {kind.lower()} identity")
    return match[1]


def has_signoff(message, author):
    trailers = git("interpret-trailers", "--parse", input=message)
    return any(
        line.partition(":")[0].lower() == "signed-off-by"
        and line.partition(":")[2].strip().casefold() == author.casefold()
        for line in trailers.splitlines()
    )


def prepare_message(path):
    # A committer may attest only their own contribution. An upstream author's
    # missing attestation is never manufactured by the hook.
    committer = identity("COMMITTER")
    message = Path(path).read_text()
    if not has_signoff(message, committer):
        git("interpret-trailers", "--in-place", "--trailer",
            f"Signed-off-by: {committer}", path)


def check_message(path):
    author = identity("AUTHOR")
    if not has_signoff(Path(path).read_text(), author):
        raise ValueError(f"Missing author trailer: Signed-off-by: {author}")


def check_signing_config():
    if git("config", "--get", "commit.gpgsign").strip().lower() != "true":
        raise ValueError("Enable commit.gpgsign with tools/install_git_hooks.py")
    if git("config", "--get", "gpg.format").strip() != "openpgp":
        raise ValueError("Use OpenPGP signing with tools/install_git_hooks.py")
    if not git("config", "--get", "user.signingkey").strip():
        raise ValueError("Configure your existing OpenPGP signing key")


def check_commit(sha, repository=None, *, incidents=(), policy_repository=None):
    author, message = git("show", "-s", "--format=%an <%ae>%x00%B", sha).split("\0", 1)
    incident = None
    if not has_signoff(message, author):
        incident = next((item for item in incidents
                         if (item["repository"], item["commit"], item["author"])
                         == (policy_repository, sha, author)), None)
        if incident is None:
            raise ValueError(f"{sha}: missing author Signed-off-by: {author}")
    if repository:
        record = json.loads(run("gh", "api", f"repos/{repository}/commits/{sha}"))
        verification = record.get("commit", {}).get("verification", {})
        if record.get("sha") != sha or verification.get("verified") is not True:
            raise ValueError(f"{sha}: GitHub signature verification failed ({verification.get('reason')})")
        if not (verification.get("signature") or "").startswith("-----BEGIN PGP SIGNATURE-----"):
            raise ValueError(f"{sha}: a verified OpenPGP signature is required")
    else:
        headers = git("cat-file", "commit", sha).split("\n\n", 1)[0]
        if "\ngpgsig -----BEGIN PGP SIGNATURE-----\n" not in "\n" + headers:
            raise ValueError(f"{sha}: an OpenPGP commit signature is required")
        # This verifies the cryptographic signature, not merely its presence.
        # GitHub web merges require the web-flow public key in the local keyring;
        # alternatively audit published commits with --github-repository.
        git("-c", "gpg.format=openpgp", "verify-commit", sha)
    return incident


def pre_push(lines):
    updates = []
    for line in lines:
        fields = line.split()
        if len(fields) != 4:
            raise ValueError("Invalid pre-push input")
        local_ref, new, remote_ref, old = fields
        if remote_ref in PROTECTED:
            raise ValueError(f"Direct changes to {remote_ref} are forbidden; merge a checked PR")
        if not all(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", oid) for oid in (new, old)):
            raise ValueError("Invalid pre-push object ID")
        if remote_ref.startswith("refs/tags/") and set(old) != {"0"}:
            raise ValueError(f"Existing tag {remote_ref} is immutable")
        updates.append((local_ref, new, remote_ref, old))
    checked = set()
    for _, new, remote_ref, old in updates:
        if set(new) == {"0"}:
            continue
        if remote_ref.startswith("refs/tags/"):
            if git("cat-file", "-t", new).strip() != "tag":
                raise ValueError("Tags must be annotated and OpenPGP signed")
            tag = git("cat-file", "tag", new)
            if "-----BEGIN PGP SIGNATURE-----" not in tag:
                raise ValueError("Tags must be OpenPGP signed")
            git("-c", "gpg.format=openpgp", "verify-tag", new)
        elif not remote_ref.startswith("refs/heads/"):
            raise ValueError(f"Unsupported public reference: {remote_ref}")
        # Audit new history. Published ancestors are covered by server rules and
        # the full-history audit; avoid needing every contributor's key locally.
        revisions = git("rev-list", new, "--not", "--remotes").splitlines()
        for sha in revisions:
            if sha not in checked:
                check_commit(sha)
                checked.add(sha)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--range", metavar="REV", help="Git revision or BASE..HEAD range")
    mode.add_argument("--head", metavar="REV", help="Audit all ancestors of one head (identical PR/push CI scope)")
    mode.add_argument("--all", action="store_true", help="Audit local branches and tags")
    mode.add_argument("--prepare-message", metavar="PATH")
    mode.add_argument("--check-message", metavar="PATH")
    mode.add_argument("--check-config", action="store_true")
    mode.add_argument("--pre-push", action="store_true")
    parser.add_argument("--github-repository", metavar="OWNER/REPO",
                        help="Use GitHub's recorded cryptographic verification for published commits")
    args = parser.parse_args()
    if args.github_repository and not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.github_repository):
        raise ValueError("Invalid GitHub repository")
    if args.prepare_message:
        prepare_message(args.prepare_message)
    elif args.check_message:
        check_message(args.check_message)
    elif args.check_config:
        check_signing_config()
    elif args.pre_push:
        pre_push(sys.stdin)
    else:
        revision = args.head or args.range
        if revision and revision.startswith("-"):
            raise ValueError("A revision must not be an option")
        incidents = historical_incidents()
        policy_repository = json.loads((ROOT / "repository-settings.json").read_text())["repository"]
        if args.github_repository and args.github_repository != policy_repository:
            raise ValueError("Signature repository must match the local policy repository")
        if args.head:
            # Resolve to a single commit; a BASE..HEAD range is not a head.
            revision = git("rev-parse", "--verify", "--end-of-options", f"{args.head}^{{commit}}").strip()
        options = ["--branches", "--tags"] if args.all else ["--end-of-options", revision, "--"]
        commits = git("rev-list", *options).splitlines()
        failures = []
        recorded = []
        for sha in commits:
            try:
                incident = check_commit(sha, args.github_repository,
                                        incidents=incidents, policy_repository=policy_repository)
                if incident:
                    recorded.append(sha)
                    print(f"HISTORICAL DCO VIOLATION (unresolved): {policy_repository}@{sha}; "
                          "missing author signoff; recorded policy exception, not certification")
            except ValueError as exc:
                failures.append(str(exc))
        if failures:
            raise ValueError("\n".join(failures))
        print(f"OK: {len(commits)} commits audited; {len(commits)-len(recorded)} author sign-offs; "
              f"{len(recorded)} recorded historical DCO violations; all signatures verified OpenPGP")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, json.JSONDecodeError) as exc:
        print(f"commit policy: {exc}", file=sys.stderr)
        raise SystemExit(1)
