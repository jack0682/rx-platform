#!/usr/bin/env python3
"""Generate public adversarial release fixtures from a reviewed custody copy."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from development_release_custody import decrypt_envelope, public_key, validate_envelope
from sign_release import KEY_ID, CHANNEL, canonical, signed


def write_new(path, value):
    if path.exists() or path.is_symlink():
        raise ValueError("fixture output already exists")
    path.write_bytes(canonical(value) + b"\n")


def release(manifest, signature):
    return {"manifest": manifest, "signature": signature}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--encrypted-key", required=True, type=Path)
    parser.add_argument("--custody-record", required=True, type=Path)
    parser.add_argument("--inventory", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    validate_envelope(args.encrypted_key)
    custody = json.loads(args.custody_record.read_text())
    private = decrypt_envelope(args.encrypted_key)
    if custody.get("key_id") != KEY_ID or public_key(private).hex() != custody.get(
        "public_key"
    ):
        parser.error("reviewed custody key required")
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "schema": "rx.release.v1",
        "channel": CHANNEL,
        "version": "1",
        "inventory_sha256": hashlib.sha256(args.inventory.read_bytes()).hexdigest(),
    }
    foreign = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ED25519"])
    cases = {
        "foreign-key-normal-id.json": release(
            manifest, signed(foreign, "RX-RELEASE-v1", manifest)
        ),
        "normal-key-wrong-id.json": release(
            manifest,
            signed(private, "RX-RELEASE-v1", manifest, "attacker/wrong-id"),
        ),
        "release-under-revocation-domain.json": release(
            manifest, signed(private, "RX-RELEASE-REVOCATIONS-v1", manifest)
        ),
    }
    zero = {**manifest, "version": "0"}
    maximum = {**manifest, "version": str(2**64 - 1)}
    cases["version-zero.json"] = release(zero, signed(private, "RX-RELEASE-v1", zero))
    cases["version-max.json"] = release(
        maximum, signed(private, "RX-RELEASE-v1", maximum)
    )
    for name, value in cases.items():
        write_new(args.output / name, value)
    print(
        json.dumps(
            {
                "schema": "rx.release-rotation-fixtures.v1",
                "files": sorted(cases),
                "private_bytes_emitted": False,
            }
        )
    )


if __name__ == "__main__":
    main()
