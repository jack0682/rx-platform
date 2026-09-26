#!/usr/bin/env python3
"""Throw rollback, revocation, content and freshness attacks at a signed image."""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--old-image", required=True)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--metadata", required=True, type=Path)
    parser.add_argument("--old-metadata", required=True, type=Path)
    parser.add_argument("--runtime-root-file", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.image])
    )[0]["Id"]
    old_image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.old_image])
    )[0]["Id"]
    rows = []

    def run(
        label,
        *,
        selected=image,
        command="inspect",
        metadata=None,
        inventory=None,
        state=None,
        environment=None,
    ):
        subprocess.run(["docker", "ps"], check=True, capture_output=True)
        output = evidence / label
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        argv = [
            "docker",
            "run",
            "--rm",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--user",
            "10001:10001",
            "--network",
            "none",
            "--tmpfs",
            "/tmp:rw,uid=10001,gid=10001",
            "--tmpfs",
            "/run/rx-solutions:rw,uid=10001,gid=10001",
            "-v",
            str(args.config.resolve()) + ":/config.json:ro",
        ]
        if state:
            state.mkdir(mode=0o777, parents=True, exist_ok=True)
            state.chmod(0o777)
            argv += ["-v", str(state.resolve()) + ":/var/lib/rx-solutions"]
        else:
            argv += ["--tmpfs", "/var/lib/rx-solutions:rw,uid=10001,gid=10001"]
        if metadata:
            argv += [
                "-v",
                str((metadata / "release.json").resolve())
                + ":/opt/rx/manifests/release.json:ro",
                "-v",
                str((metadata / "revocations.json").resolve())
                + ":/opt/rx/manifests/revocations.json:ro",
            ]
        if inventory:
            argv += [
                "-v",
                str(inventory.resolve()) + ":/opt/rx/manifests/runtime-files.json:ro",
            ]
        for key, value in (environment or {}).items():
            argv += ["-e", f"{key}={value}"]
        argv += [
            "--entrypoint",
            "/opt/rx/bin/rx-solutionsd",
            selected,
            command,
            "/config.json",
        ]
        (output / "command.json").write_text(json.dumps(argv, indent=2) + "\n")
        result = subprocess.run(argv, capture_output=True, text=True, timeout=60)
        row = {
            "case": label,
            "exit": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }
        (output / "result.json").write_text(json.dumps(row, indent=2) + "\n")
        rows.append(row)
        print(json.dumps({"case": label, "exit": result.returncode}), flush=True)
        return row

    state = evidence / "retained-state"
    positive = run("new-root-positive", command="init", state=state)
    lower = run(
        "version-rollback",
        command="init",
        metadata=args.metadata / "v12",
        inventory=args.metadata / "runtime-files.json",
        state=state,
    )
    equivocation = run(
        "same-version-different-inventory",
        command="init",
        metadata=args.metadata / "same-v13-compact",
        inventory=args.metadata / "runtime-files-compact.json",
        state=state,
    )
    revocation_rollback = run(
        "revocation-version-rollback",
        command="init",
        metadata=args.metadata / "rev1",
        inventory=args.metadata / "runtime-files.json",
        state=state,
    )
    revocation_shrink = run(
        "revocation-shrink",
        command="init",
        metadata=args.metadata / "rev3-shrink",
        inventory=args.metadata / "runtime-files.json",
        state=state,
    )
    revoked = run(
        "revoked-release",
        metadata=args.metadata / "v14-revoked",
        inventory=args.metadata / "runtime-files.json",
    )
    maximum = run(
        "maximum-version-inspect-only",
        metadata=args.metadata / "max",
        inventory=args.metadata / "runtime-files.json",
    )
    content = run(
        "post-sign-inventory-swap",
        metadata=args.metadata / "signed",
        inventory=args.metadata / "runtime-files-compact.json",
    )
    old_to_new = run(
        "old-binary-new-metadata-integration",
        selected=old_image,
        metadata=args.metadata / "signed",
        inventory=args.metadata / "runtime-files.json",
    )
    injected = run(
        "runtime-root-injection-integration",
        metadata=args.old_metadata,
        inventory=args.old_metadata / "runtime-files.json",
        environment={
            "RX_RELEASE_KEY_ID": "rx/development-release-2026-09",
            "RX_RELEASE_PUBLIC_KEY": "e62da4e29436cdcde5b57a2f8ee44170eee3b10fbc0cea311d0414341bee1e35",
            "RX_RELEASE_ROOT_FILE": str(args.runtime_root_file.resolve()),
        },
    )
    fresh_a = run(
        "offline-freshness-fresh-state-a",
        command="init",
        state=evidence / "fresh-state-a",
    )
    fresh_b = run(
        "offline-freshness-fresh-state-b",
        command="init",
        state=evidence / "fresh-state-b",
    )
    fresh_a_lower = run(
        "offline-freshness-floor-a",
        command="init",
        metadata=args.metadata / "v12",
        inventory=args.metadata / "runtime-files.json",
        state=evidence / "fresh-state-a",
    )
    fresh_b_lower = run(
        "offline-freshness-floor-b",
        command="init",
        metadata=args.metadata / "v12",
        inventory=args.metadata / "runtime-files.json",
        state=evidence / "fresh-state-b",
    )

    # Release admission commits before the later service-configuration gate.
    # The following lower-version refusal proves that the floor was recorded.
    assert (
        positive["exit"] != 0
        and "init requires named Host/Executor" in positive["stderr"]
    )
    for item in [lower, equivocation, revocation_rollback, revocation_shrink]:
        assert (
            item["exit"] != 0 and "release/rollback" in item["stdout"] + item["stderr"]
        )
    assert (
        revoked["exit"] != 0
        and "release/revoked" in revoked["stdout"] + revoked["stderr"]
    )
    assert maximum["exit"] == 0
    assert (
        content["exit"] != 0
        and "release/content-mismatch" in content["stdout"] + content["stderr"]
    )
    # Source pinning precedes release verification in the composed old/new-image
    # cross attacks. The isolated verifier probe records the root-specific
    # unknown-key result; these integration rows preserve the earlier boundary.
    for item in [old_to_new, injected]:
        assert (
            item["exit"] != 0
            and "release/content-mismatch" in item["stdout"] + item["stderr"]
        )
    for item in [fresh_a, fresh_b]:
        assert (
            item["exit"] != 0 and "init requires named Host/Executor" in item["stderr"]
        )
    for item in [fresh_a_lower, fresh_b_lower]:
        assert (
            item["exit"] != 0 and "release/rollback" in item["stdout"] + item["stderr"]
        )
    summary = {
        "schema": "rx.release-rotation-runtime-attacks.v1",
        "image": image,
        "old_image": old_image,
        "results": [{"case": row["case"], "exit": row["exit"]} for row in rows],
        "offline_revocation_freshness": "NOT_ESTABLISHED",
        "fresh_state_replay_accepted_twice": True,
        "physical_qualification": "NOT_PERFORMED",
    }
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
