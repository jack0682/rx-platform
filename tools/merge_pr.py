#!/usr/bin/env python3
"""Merge an exact, checked PR head through GitHub with an explicit DCO trailer."""
import argparse
import json
from pathlib import Path
import re
import sys

from check_commit_policy import has_signoff, run


def api(path, payload=None):
    command = ["gh", "api", path]
    if payload is not None:
        command += ["--method", "PUT", "--input", "-"]
    return json.loads(run(*command, input=json.dumps(payload) if payload is not None else None))


def require_checks(repository, number, head):
    checks = api(f"repos/{repository}/commits/{head}/check-runs?filter=latest&per_page=100")
    if checks["total_count"] > 100:
        raise ValueError("More than 100 checks; inspect the PR on GitHub before merging")
    for name, application in (("CI", 15368), ("DCO", 1861)):
        matches = [check for check in checks["check_runs"]
                   if check["name"] == name and check.get("app", {}).get("id") == application]
        latest = max(matches, key=lambda check: check["id"], default={})
        if (latest.get("head_sha") != head or latest.get("status") != "completed"
                or latest.get("conclusion") != "success"):
            raise ValueError(f"{name} from application {application} must succeed for {head}")
        if name == "CI":
            match = re.fullmatch(r"https://github\.com/" + re.escape(repository)
                                 + r"/actions/runs/(\d+)/job/\d+", latest.get("details_url", ""))
            if not match:
                raise ValueError("CI does not identify a GitHub Actions run in this repository")
            workflow = api(f"repos/{repository}/actions/runs/{match[1]}")
            if (workflow.get("event") != "pull_request" or workflow.get("head_sha") != head
                    or workflow.get("path") != ".github/workflows/ci.yml"
                    or not any(pr.get("number") == number for pr in workflow.get("pull_requests", []))):
                raise ValueError("CI must belong to this PR, exact head and repository workflow")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("number", type=int)
    parser.add_argument("--signoff", help="Your GitHub web commit Name <email>; defaults to your no-reply identity")
    args = parser.parse_args()
    repository = json.loads((Path(__file__).resolve().parents[1] / "repository-settings.json").read_text())["repository"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository) or args.number < 1:
        raise ValueError("Invalid repository or PR number")
    prefix = f"repos/{repository}"
    pr = api(f"{prefix}/pulls/{args.number}")
    if (pr["state"] != "open" or pr["draft"] or pr["base"]["ref"] not in ("main", "develop")
            or pr.get("mergeable") is not True or pr.get("mergeable_state") != "clean"):
        raise ValueError("PR must be open, ready, conflict-free and current with all repository rules")
    head = pr["head"]["sha"]
    require_checks(repository, args.number, head)
    actor = api("user")
    signoff = args.signoff or f"{actor['name'] or actor['login']} <{actor['id']}+{actor['login']}@users.noreply.github.com>"
    if not re.fullmatch(r"[^<>\r\n]+ <[^<>\s]+>", signoff):
        raise ValueError("Sign-off must be your GitHub web commit Name <email>")
    # GitHub checks the SHA atomically and enforces the live rules. This endpoint
    # has no administrator override. Merge commits preserve GitFlow ancestry.
    result = api(f"{prefix}/pulls/{args.number}/merge", {
        "sha": head, "merge_method": "merge",
        "commit_title": f"Merge pull request #{args.number}: {pr['title']}",
        "commit_message": f"Signed-off-by: {signoff}\n",
    })
    if result.get("merged") is not True:
        raise ValueError(f"GitHub did not merge the PR: {result.get('message')}")
    commit = api(f"{prefix}/commits/{result['sha']}")["commit"]
    verification = commit.get("verification", {})
    author = f"{commit['author']['name']} <{commit['author']['email']}>"
    if (verification.get("verified") is not True
            or not (verification.get("signature") or "").startswith("-----BEGIN PGP SIGNATURE-----")
            or not has_signoff(commit["message"], author)):
        raise ValueError(f"PR merged as {result['sha']}, but post-merge signature/DCO verification failed; inspect immediately")
    print(f"Merged {repository}#{args.number}: {result['sha']} (verified OpenPGP and author DCO)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError) as exc:
        print(f"PR merge: {exc}", file=sys.stderr)
        raise SystemExit(1)
