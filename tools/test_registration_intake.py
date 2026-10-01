#!/usr/bin/env python3
"""Actual S source writer/fence -> authenticated P intake; no equipment or process adoption."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("solutions", "source-scene", "prior-reader", "platform-target", "solutions-target", "evidence"):
        parser.add_argument(f"--{option}", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    solutions = args.solutions.resolve()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    scene = args.source_scene.resolve()
    if json.loads((scene / "result.json").read_text())["result"] != "PASS_REPORTER_UNAVAILABLE_LOCAL_STOP":
        raise SystemExit("a completed quiescent source scene is required")
    source = scene / "data/managed/registration.db"
    prior_reader = args.prior_reader.resolve()
    if not source.is_file() or not prior_reader.is_file():
        raise SystemExit("source and independently preserved prior reader are required")
    prior = json.loads(prior_reader.with_name("source.json").read_text())
    if prior["reader_format_max"] != 7 or prior["sha256"] != hashlib.sha256(prior_reader.read_bytes()).hexdigest():
        raise SystemExit("prior reader identity differs from its preserved provenance")

    def inventory():
        return {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in source.parent.glob("registration.db*") if path.is_file()
        }

    before = inventory()

    def run(repo, label, command, environment):
        with (evidence / f"{label}.log").open("w") as output:
            result = subprocess.run(
                [str(repo / "tools/cargo"), *command], cwd=repo,
                env=dict(os.environ, **environment), stdout=output, stderr=subprocess.STDOUT,
            )
        if result.returncode:
            raise SystemExit(f"{label} failed; retained log")

    run(solutions, "source-build", [
        "build", "-p", "rx-supervisor", "--features", "test-harness",
        "--bin", "rx-registration-source-fixture", "--locked",
        "--target-dir", str(args.solutions_target.resolve()),
    ], {})
    run(root, "actual-intake", [
        "test", "-p", "rx-api", "--test", "resident_reporting",
        "actual_registration_source_is_imported_through_authenticated_http_and_original_request_recovery",
        "--locked", "--target-dir", str(args.platform_target.resolve()), "--", "--ignored", "--exact",
    ], {
        "RX_REGISTRATION_SOURCE": str(source),
        "RX_REGISTRATION_SOURCE_FIXTURE": str(args.solutions_target.resolve() / "debug/rx-registration-source-fixture"),
        "RX_REGISTRATION_PRIOR_READER": str(prior_reader),
        "RX_REGISTRATION_INTAKE_EVIDENCE": str(evidence / "result.json"),
    })
    if before != inventory():
        raise SystemExit("original source inventory changed")
    result = json.loads((evidence / "result.json").read_text())
    if result["status"] != "PASS_TARGET_INTAKE":
        raise SystemExit("target intake did not pass")
    sources = {}
    for name, repo in [("platform", root), ("solutions", solutions)]:
        sources[name] = {
            "commit": subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip(),
            "changes": subprocess.check_output(["git", "-C", str(repo), "status", "--porcelain"], text=True).splitlines(),
        }
    (evidence / "scope.json").write_text(json.dumps({
        "sources": sources, "source_unchanged": True, "source_inventory": before,
        "prior_reader": prior,
        "scope": "actual S source, no-Cell P writer and HTTP route; no process ownership or execution assignment",
    }, indent=2) + "\n")
    print(json.dumps({"status": "PASS_TARGET_INTAKE", "evidence": str(evidence)}))


if __name__ == "__main__":
    main()
