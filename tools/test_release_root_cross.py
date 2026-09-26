#!/usr/bin/env python3
"""Build old/new-root verifiers and throw cross-root and runtime-input attacks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CARGO = ROOT / "tools/cargo"
FIXTURES = ROOT / "crates/rx-package/tests/fixtures/release"
OLD_PUBLIC = """[
    0xe6, 0x2d, 0xa4, 0xe2, 0x94, 0x36, 0xcd, 0xcd, 0xe5, 0xb5, 0x7a, 0x2f, 0x8e, 0xe4, 0x41, 0x70,
    0xee, 0xe3, 0xb1, 0x0f, 0xbc, 0x0c, 0xea, 0x31, 0x1d, 0x04, 0x14, 0x34, 0x1b, 0xee, 0x1e, 0x35,
]"""


def copy_source(destination):
    shutil.copytree(
        ROOT,
        destination,
        ignore=shutil.ignore_patterns(".git", "target", "node_modules", "__pycache__"),
    )


def old_root(source):
    path = source / "crates/rx-package/src/release/root.rs"
    text = path.read_text().replace(
        'pub const KEY_ID: &str = "rx/development-release-2026-09-r2";',
        'pub const KEY_ID: &str = "rx/development-release-2026-09";',
    )
    text, count = re.subn(
        r"pub const PUBLIC_KEY: \[u8; 32\] = \[.*?\n\];",
        "pub const PUBLIC_KEY: [u8; 32] = " + OLD_PUBLIC + ";",
        text,
        count=1,
        flags=re.S,
    )
    if count != 1:
        raise RuntimeError("old root mutation anchor")
    path.write_text(text)


def build(source, target, environment=None):
    command = [
        str(CARGO),
        "build",
        "--manifest-path",
        str(source / "Cargo.toml"),
        "-p",
        "rx-package",
        "--example",
        "release_probe",
        "--locked",
        "--target-dir",
        str(target),
    ]
    subprocess.run(command, check=True, capture_output=True, env=environment)
    return target / "debug/examples/release_probe"


def invoke(binary, group, environment=None):
    paths = [
        FIXTURES / group / "release.json",
        FIXTURES / group / "revocations.json",
        FIXTURES / group / "inventory.json",
        FIXTURES / group / "payload",
    ]
    result = subprocess.run(
        [str(binary), *(str(path) for path in paths)],
        capture_output=True,
        text=True,
        env=environment,
    )
    return {"exit": result.returncode, "stdout": result.stdout.strip()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--runtime-root-file", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    root_hash = hashlib.sha256(
        (ROOT / "crates/rx-package/src/release/root.rs").read_bytes()
    ).hexdigest()
    with tempfile.TemporaryDirectory(prefix="rx-root-cross-") as temporary:
        temporary = Path(temporary)
        old_source = temporary / "old-source"
        copy_source(old_source)
        old_root(old_source)
        old_binary = build(old_source, temporary / "old-target")
        new_binary = build(ROOT, temporary / "new-target")
        injected = os.environ.copy()
        injected.update(
            RX_RELEASE_KEY_ID="rx/development-release-2026-09",
            RX_RELEASE_PUBLIC_KEY="e62da4e29436cdcde5b57a2f8ee44170eee3b10fbc0cea311d0414341bee1e35",
            RX_RELEASE_ROOT_FILE=str(args.runtime_root_file.resolve()),
        )
        build_environment = injected.copy()
        build_environment["RUSTFLAGS"] = "--cfg rx_release_root_from_environment"
        build_env_binary = build(
            ROOT, temporary / "build-env-target", build_environment
        )
        rows = {
            "old_binary_old_release": invoke(old_binary, "legacy-root"),
            "old_binary_new_release": invoke(old_binary, "v1"),
            "new_binary_new_release": invoke(new_binary, "v1"),
            "new_binary_old_release": invoke(new_binary, "legacy-root"),
            "runtime_inputs_new_release": invoke(new_binary, "v1", injected),
            "runtime_inputs_old_release": invoke(new_binary, "legacy-root", injected),
            "build_inputs_new_release": invoke(build_env_binary, "v1", injected),
            "build_inputs_old_release": invoke(
                build_env_binary, "legacy-root", injected
            ),
        }
    (evidence / "observed.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(json.dumps({"observed": rows}), flush=True)
    assert rows["old_binary_old_release"]["exit"] == 0
    assert rows["new_binary_new_release"]["exit"] == 0
    for name in [
        "old_binary_new_release",
        "new_binary_old_release",
        "runtime_inputs_old_release",
        "build_inputs_old_release",
    ]:
        assert rows[name] == {
            "exit": 3,
            "stdout": "refused condition=release/unknown-key",
        }, (name, rows[name])
    assert rows["runtime_inputs_new_release"]["exit"] == 0
    assert rows["build_inputs_new_release"]["exit"] == 0
    if (
        hashlib.sha256(
            (ROOT / "crates/rx-package/src/release/root.rs").read_bytes()
        ).hexdigest()
        != root_hash
    ):
        raise RuntimeError("product root changed during cross-root probe")
    result = {
        "schema": "rx.release-root-cross-attacks.v1",
        "rows": rows,
        "runtime_inputs_ignored": True,
        "build_environment_cfg_ignored": True,
        "dual_root_window": False,
    }
    (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
