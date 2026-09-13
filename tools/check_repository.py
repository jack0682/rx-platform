#!/usr/bin/env python3
"""Check repository-owned links, JSON, pinned contracts and PR branch policy.

Uses only the Python standard library. External URLs and Markdown fragments are
not fetched; sibling-checkout links are reported separately from local targets.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def repository_files(root):
    output = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root,
    )
    return sorted({root / os.fsdecode(p) for p in output.split(b"\0") if p})


def local_link_errors(root, path):
    source = path.read_text(encoding="utf-8")
    source = re.sub(r"(?ms)^\s*(`{3,}|~{3,}).*?^\s*\1\s*$", "", source)
    source = re.sub(r"`[^`\n]+`", "", source)
    targets = re.findall(r"\[[^\]\n]*\]\(\s*(<[^>]+>|[^\s)]+)", source)
    targets += re.findall(r"(?m)^\s*\[(?!\^)[^\]\n]+\]:\s*(<[^>]+>|\S+)", source)
    errors, siblings = [], 0
    for target in targets:
        parsed = urlsplit(target.strip("<>"))
        decoded = unquote(parsed.path)
        if parsed.scheme == "file" or (not parsed.scheme and decoded.startswith(("/Users/", "/home/"))):
            errors.append(f"{path.relative_to(root)}: machine-specific link target {target}")
            continue
        if parsed.scheme or parsed.netloc or not parsed.path:
            continue
        resolved = (path.parent / decoded).resolve()
        if not resolved.is_relative_to(root):
            siblings += 1
        elif not resolved.exists():
            errors.append(f"{path.relative_to(root)}: missing link target {target}")
    return errors, siblings


def contract_errors(folder):
    manifest_path = folder / "protocol_manifest.json"
    manifest = read_json(manifest_path)
    errors = []
    documents = manifest.get("normative_document_sha256", {})
    if len(documents) != 4:
        errors.append(f"{folder}: expected four normative document hashes")
    for name, expected in documents.items():
        path = (folder / name).resolve()
        if not path.is_relative_to(folder.resolve()) or not path.is_file():
            errors.append(f"{folder}: missing or invalid normative path {name}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            errors.append(f"{folder}: normative hash differs for {name}")
    integrity = read_json(folder / "manifest_integrity.json")
    expected = integrity.get("schema_hash_sha256", integrity.get("cell_manifest_sha256"))
    # These manifests are already stored as JCS bytes. Hash the exact artifact;
    # do not silently reserialize it using a non-JCS JSON implementation.
    if hashlib.sha256(manifest_path.read_bytes()).hexdigest() != expected:
        errors.append(f"{folder}: manifest integrity hash differs")
    return errors


def sdk_errors(root, files):
    folder = root / "sdk"
    lock = read_json(folder / "source-lock.json")
    actual = {
        str(path.relative_to(folder)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in files
        if path.is_file() and path.is_relative_to(folder)
        and path != folder / "source-lock.json"
    }
    if lock.get("schema") != "rx.host-sdk.v1" or lock.get("files") != actual:
        return ["sdk/source-lock.json differs from the repository SDK inventory"]
    return []


def branch_error(event):
    pr = event.get("pull_request")
    if not isinstance(pr, dict):
        return "pull_request event payload is missing"
    base, head = pr.get("base", {}), pr.get("head", {})
    target, source = base.get("ref", ""), head.get("ref", "")
    base_repo = (base.get("repo") or {}).get("full_name")
    head_repo = (head.get("repo") or {}).get("full_name")
    same_repo = bool(base_repo) and head_repo == base_repo
    named = lambda prefix: source.startswith(prefix + "/") and len(source) > len(prefix) + 1
    # Bots can prepare version updates for develop and security fixes for main.
    # This permits a branch route; it never grants approval or merge permission.
    if target in {"main", "develop"} and named("dependabot"):
        return None
    if target == "develop":
        if any(named(prefix) for prefix in ("feature", "fix", "docs", "chore", "codex")):
            return None
        if same_repo and (source == "main" or named("release") or named("hotfix")):
            return None
    if target == "main":
        if same_repo and (source == "develop" or named("release") or named("hotfix")):
            return None
    return f"unsupported PR route {source!r} -> {target!r}; see CONTRIBUTING.md"


def check(root):
    files = [p for p in repository_files(root) if p.is_file()]
    errors, json_count, link_count, siblings = [], 0, 0, 0
    for name in ("LICENSE", "NOTICE"):
        if not (root / name).is_file():
            errors.append(f"Required project license file is missing: {name}")
    license_path = root / "LICENSE"
    if license_path.is_file():
        license_text = license_path.read_text(encoding="utf-8")
        if "Apache License" not in license_text or "Version 2.0" not in license_text:
            errors.append("LICENSE must contain the selected Apache License 2.0 text")
    for path in files:
        try:
            if path.suffix == ".json":
                read_json(path)
                json_count += 1
            if path.suffix == ".md":
                found, outside = local_link_errors(root, path)
                errors.extend(found)
                siblings += outside
                link_count += 1
        except (OSError, ValueError) as error:
            errors.append(f"{path.relative_to(root)}: {error}")
    spec = root / ("docs" if (root / "docs/contracts").is_dir() else "spec")
    if (root / "sdk").is_dir():
        spec = root / "sdk/spec"
        errors.extend(sdk_errors(root, files))
    for family in ("contracts", "cell_operations"):
        try:
            errors.extend(contract_errors(spec / family / "v1.0"))
        except (OSError, ValueError, KeyError) as error:
            errors.append(f"{family}: {error}")
    print(f"Checked {json_count} JSON files, local targets in {link_count} Markdown files, "
          f"and two contract baselines; {siblings} links outside this checkout are not checked.")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", type=Path, help="validate a pull_request event JSON file")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if args.event:
        error = branch_error(read_json(args.event))
        errors = [error] if error else []
    else:
        errors = check(root)
    if errors:
        raise SystemExit("\n".join(errors))
    print("Repository checks passed.")


if __name__ == "__main__":
    main()
