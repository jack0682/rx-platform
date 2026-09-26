#!/usr/bin/env python3
"""Offline development-release signing from a reviewed encrypted envelope.

The runtime has no signing/key input. Requires GnuPG and OpenSSL Ed25519 support.
This authoring tool does not establish product release custody or qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from development_release_custody import (
    decrypt_envelope,
    public_key,
    validate_envelope,
)

KEY_ID = "rx/development-release-2026-09-r2"
CHANNEL = "rx/solutions-development"


def canonical(value):
    # This schema has only ASCII identifiers, hex digests and decimal strings;
    # sorted compact JSON equals the Rust JCS bytes for this restricted domain.
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode()


def signed(private_pem, domain, value, key_id=KEY_ID):
    message = domain.encode() + b"\0" + key_id.encode() + b"\0" + canonical(value)
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "message"
        path.write_bytes(message)
        signature = subprocess.check_output(
            [
                "openssl",
                "pkeyutl",
                "-sign",
                "-rawin",
                "-inkey",
                "/dev/stdin",
                "-in",
                str(path),
            ],
            input=private_pem,
        )
    return {"key": key_id, "signature": signature.hex()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--encrypted-key", required=True, type=Path)
    p.add_argument("--custody-record", required=True, type=Path)
    p.add_argument("--inventory", required=True, type=Path)
    p.add_argument("--version", required=True, type=int)
    p.add_argument("--revocation-version", required=True, type=int)
    p.add_argument("--revoke", action="append", default=[])
    p.add_argument("--output", required=True, type=Path)
    a = p.parse_args()
    if not (0 < a.version < 2**64 and 0 < a.revocation_version < 2**64):
        p.error("versions must be positive u64 values")
    if any(
        len(d) != 64 or any(c not in "0123456789abcdef" for c in d) for d in a.revoke
    ):
        p.error("revoked identities must be lowercase SHA256 digests")
    try:
        validate_envelope(a.encrypted_key)
    except (OSError, ValueError) as error:
        p.error(str(error))
    custody = json.loads(a.custody_record.read_text())
    if (
        custody.get("schema") != "rx.development-release-custody.v1"
        or custody.get("key_id") != KEY_ID
        or custody.get("development_signing_custody")
        != "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL"
    ):
        p.error("current reviewed custody record required")
    envelope_hash = hashlib.sha256(a.encrypted_key.read_bytes()).hexdigest()
    if envelope_hash not in custody.get("envelope_sha256", {}).values():
        p.error("encrypted key is not a reviewed custody envelope")
    private_pem = decrypt_envelope(a.encrypted_key)
    if public_key(private_pem).hex() != custody.get("public_key"):
        p.error("encrypted key does not match reviewed custody root")
    manifest = {
        "schema": "rx.release.v1",
        "channel": CHANNEL,
        "version": str(a.version),
        "inventory_sha256": hashlib.sha256(a.inventory.read_bytes()).hexdigest(),
    }
    revocations = {
        "schema": "rx.release-revocations.v1",
        "channel": CHANNEL,
        "version": str(a.revocation_version),
        "revoked": sorted(set(a.revoke)),
    }
    a.output.mkdir(parents=True, exist_ok=True)
    for filename, value, field, domain in [
        ("release.json", manifest, "manifest", "RX-RELEASE-v1"),
        ("revocations.json", revocations, "revocations", "RX-RELEASE-REVOCATIONS-v1"),
    ]:
        output = a.output / filename
        if output.exists():
            p.error(f"output already exists: {output}")
        output.write_bytes(
            canonical({field: value, "signature": signed(private_pem, domain, value)})
            + b"\n"
        )
    print(
        json.dumps(
            {
                "release_digest": hashlib.sha256(
                    b"RX-RELEASE-IDENTITY-v1\n" + canonical(manifest)
                ).hexdigest(),
                "development_signing_custody": "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL",
                "product_signing_custody": "NOT_ESTABLISHED",
            }
        )
    )


if __name__ == "__main__":
    main()
