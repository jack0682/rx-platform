#!/usr/bin/env python3
"""Verify both SDK self-integrity and byte-for-byte agreement with current platform export."""
from pathlib import Path
import argparse
import hashlib
import json
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("destination", type=Path)
args = parser.parse_args()
if args.destination.is_symlink():
    raise SystemExit("SDK destination symlink refused")
destination = args.destination.resolve()

def inventory(directory):
    values = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise SystemExit(f"SDK symlink refused: {path}")
        relative = str(path.relative_to(directory))
        if path.is_file() and relative != "source-lock.json":
            values[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
        elif not path.is_dir() and not path.is_file():
            raise SystemExit(f"Non-regular SDK entry refused: {path}")
    return values

lock_path = destination / "source-lock.json"
if not lock_path.is_file():
    raise SystemExit("SDK source-lock.json is missing")
lock = json.loads(lock_path.read_text())
actual = inventory(destination)
if lock.get("schema") != "rx.host-sdk.v1" or lock.get("files") != actual:
    raise SystemExit("SDK differs from its own recorded inventory")
with tempfile.TemporaryDirectory(prefix="rx-sdk-check-") as temp:
    expected_dir = Path(temp) / "sdk"
    result = subprocess.run([sys.executable, str(Path(__file__).with_name("export_host_sdk.py")), str(expected_dir)], capture_output=True, text=True)
    if result.returncode:
        raise SystemExit(result.stderr or result.stdout)
    expected = inventory(expected_dir)
changed = sorted(p for p in actual.keys() | expected.keys() if actual.get(p) != expected.get(p))
if changed:
    raise SystemExit("SDK is stale relative to platform sources:\n" + "\n".join(changed[:30]))
print(f"SDK synchronized with platform: {len(actual)} files; authority excluded")
