#!/usr/bin/env python3
"""Apply or audit the checked-in GitHub configuration using authenticated gh."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def api(method, path, payload=None):
    command = ["gh", "api", "--method", method, path]
    if payload is not None:
        command += ["--input", "-"]
    result = subprocess.run(
        command, input=json.dumps(payload) if payload is not None else None,
        text=True, capture_output=True, check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return json.loads(result.stdout) if result.stdout.strip() else None


def contains(actual, expected):
    if isinstance(expected, dict):
        # GitHub omits the default false parameter on branch update rules.
        if (isinstance(actual, dict) and actual.get("type") == "update"
                and "parameters" not in actual
                and expected.get("parameters") == {"update_allows_fetch_and_merge": False}):
            actual = {**actual, "parameters": {"update_allows_fetch_and_merge": False}}
        return isinstance(actual, dict) and all(
            key in actual and contains(actual[key], value)
            for key, value in expected.items()
        )
    if isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            return False
        remaining = list(actual)
        for item in expected:
            index = next((index for index, candidate in enumerate(remaining)
                          if contains(candidate, item)), None)
            if index is None:
                return False
            remaining.pop(index)
        return True
    return actual == expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Apply reviewed settings; default is read-only audit")
    args = parser.parse_args()
    config = json.loads((ROOT / "repository-settings.json").read_text())
    repo = config["repository"]
    automatic_fixes = config["dependency_updates"]["automated_security_fixes"]
    if type(automatic_fixes) is not bool:
        raise ValueError("automatic security fixes must be a boolean")
    if repo not in {"jack0682/rx_docs", "jack0682/rx-platform", "jack0682/rx-solutions"}:
        raise ValueError("Unexpected repository identity; review this script before extending its scope")
    prefix = f"repos/{repo}"
    # Both permanent branches must exist before any settings or protection change.
    for branch in ("main", "develop"):
        api("GET", f"{prefix}/branches/{branch}")
    existing = api("GET", f"{prefix}/rulesets?per_page=100")
    matched = {}
    for expected in config["rulesets"]:
        matches = [rule for rule in existing if rule["name"] == expected["name"]]
        if len(matches) > 1:
            raise ValueError(f"Ambiguous ruleset name: {expected['name']}")
        matched[expected["name"]] = matches[0]["id"] if matches else None
    if args.apply:
        api("PATCH", prefix, config["settings"])
        api("PUT", f"{prefix}/topics", {"names": config["topics"]})
        api("PUT", f"{prefix}/actions/permissions", config["actions"])
        api("PUT", f"{prefix}/actions/permissions/workflow", config["workflow_permissions"])
        api("PATCH", prefix, {"security_and_analysis": config["security"]})
        api("PUT", f"{prefix}/vulnerability-alerts")
        api("PUT" if automatic_fixes else "DELETE", f"{prefix}/automated-security-fixes")
        for expected in config["rulesets"]:
            rule_id = matched[expected["name"]]
            path = f"{prefix}/rulesets" + (f"/{rule_id}" if rule_id else "")
            result = api("PUT" if rule_id else "POST", path, expected)
            matched[expected["name"]] = result["id"]
    current = api("GET", prefix)
    observations = [
        ("repository settings", current, config["settings"]),
        ("topics", api("GET", f"{prefix}/topics")["names"], config["topics"]),
        ("actions", api("GET", f"{prefix}/actions/permissions"), config["actions"]),
        ("workflow permissions", api("GET", f"{prefix}/actions/permissions/workflow"), config["workflow_permissions"]),
        ("security", current.get("security_and_analysis", {}), config["security"]),
        ("automatic security fixes", api("GET", f"{prefix}/automated-security-fixes"),
         {"enabled": automatic_fixes}),
        ("vulnerability alerts", api("GET", f"{prefix}/vulnerability-alerts"), None),
    ]
    for expected in config["rulesets"]:
        rule_id = matched[expected["name"]]
        actual = api("GET", f"{prefix}/rulesets/{rule_id}") if rule_id else {}
        observations.append((expected["name"], actual, expected))
    failures = [label for label, actual, expected in observations if not contains(actual, expected)]
    expected_names = {rule["name"] for rule in config["rulesets"]}
    unmanaged = sorted(rule["name"] for rule in existing
                       if rule["name"].startswith("RX ") and rule["name"] not in expected_names)
    if unmanaged:
        failures.append("unrecorded RX rulesets: " + ", ".join(unmanaged))
    for label in failures:
        print(f"DRIFT: {repo}: {label}")
    if failures:
        return 1
    print(f"OK: {repo}: settings, Actions and active branch/tag rules match repository-settings.json")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, OSError, RuntimeError) as exc:
        print(f"github configuration: {exc}", file=sys.stderr)
        raise SystemExit(1)
