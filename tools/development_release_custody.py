#!/usr/bin/env python3
"""Create and verify two-device encrypted development release-key custody.

Private PEM bytes stay in process pipes and are never printed. This establishes
development custody only; it is not an HSM or product-signing ceremony.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

SCHEMA = "rx.development-release-custody.v1"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def run(command, *, input_bytes=None):
    return subprocess.run(
        command,
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    ).stdout


def public_key(private_pem):
    der = run(
        ["openssl", "pkey", "-pubout", "-outform", "DER", "-in", "/dev/stdin"],
        input_bytes=private_pem,
    )
    if len(der) < 32:
        raise ValueError("development custody public key unavailable")
    return der[-32:]


def decrypt_envelope(path):
    return run(["gpg", "--batch", "--quiet", "--decrypt", str(path)])


def validate_envelope(path):
    metadata = path.lstat()
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise ValueError("custody envelope must be a real regular file")
    if metadata.st_mode & 0o077:
        raise ValueError("custody envelope must be owner-only")


def secret_fingerprints():
    output = run(["gpg", "--batch", "--with-colons", "--list-secret-keys"])
    fingerprints = set()
    expect = False
    for raw in output.decode().splitlines():
        fields = raw.split(":")
        if fields[0] == "sec":
            expect = True
        elif fields[0] == "fpr" and expect:
            fingerprints.add(fields[9].upper())
            expect = False
    return fingerprints


def checked_parent(path):
    parent = path.parent
    if not parent.exists():
        parent.mkdir(mode=0o700, parents=True)
    metadata = parent.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or parent.is_symlink():
        raise ValueError("custody parent must be a real directory")
    if metadata.st_mode & 0o077:
        raise ValueError("custody parent must be owner-only")
    if path.exists() or path.is_symlink():
        raise ValueError("custody destination already exists")
    return metadata.st_dev


def under(path, root):
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def publish(path, content):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        try:
            path.unlink()
        except FileNotFoundError:
            pass
        raise


def encrypt(private_pem, recipients):
    command = ["gpg", "--batch", "--yes", "--trust-model", "always", "--encrypt"]
    for recipient in recipients:
        command += ["--recipient", recipient]
    return run(command, input_bytes=private_pem)


def load_record(path):
    value = json.loads(path.read_text())
    if value.get("schema") != SCHEMA:
        raise ValueError("custody record schema")
    return value


def verify(primary, backup, record):
    value = load_record(record)
    validate_envelope(primary)
    validate_envelope(backup)
    if primary.parent.stat().st_dev == backup.parent.stat().st_dev:
        raise ValueError("custody copies must use distinct filesystem devices")
    private_primary = decrypt_envelope(primary)
    private_backup = decrypt_envelope(backup)
    if private_primary != private_backup:
        raise ValueError("custody copies recover different private keys")
    public = public_key(private_primary).hex()
    if public != value["public_key"]:
        raise ValueError("custody public key mismatch")
    actual = {
        "primary": hashlib.sha256(primary.read_bytes()).hexdigest(),
        "backup": hashlib.sha256(backup.read_bytes()).hexdigest(),
    }
    if actual != value["envelope_sha256"]:
        raise ValueError("custody envelope digest mismatch")
    return {
        "schema": "rx.development-release-custody-verification.v1",
        "key_id": value["key_id"],
        "public_key": public,
        "copies_verified": 2,
        "separate_filesystems": True,
        "private_bytes_emitted": False,
        "product_signing_custody": "NOT_ESTABLISHED",
    }


def create(args):
    primary = args.primary.resolve()
    backup = args.backup.resolve()
    record = args.record.resolve()
    for path in [primary, backup]:
        if any(under(path, root) for root in args.forbid_root):
            raise ValueError("custody destination is inside a forbidden root")
    primary_device = checked_parent(primary)
    backup_device = checked_parent(backup)
    if primary_device == backup_device:
        raise ValueError("custody copies must use distinct filesystem devices")
    recipients = [value.upper() for value in args.recipient]
    if len(set(recipients)) < 2 or any(
        len(value) != 40 or any(c not in "0123456789ABCDEF" for c in value)
        for value in recipients
    ):
        raise ValueError("two distinct full OpenPGP recovery fingerprints required")
    missing = set(recipients) - secret_fingerprints()
    if missing:
        raise ValueError("recovery secret key unavailable")
    private_pem = run(["openssl", "genpkey", "-algorithm", "ED25519"])
    public = public_key(private_pem).hex()
    primary_bytes = encrypt(private_pem, recipients)
    backup_bytes = encrypt(private_pem, recipients)
    publish(primary, primary_bytes)
    try:
        publish(backup, backup_bytes)
        value = {
            "schema": SCHEMA,
            "ceremony_version": "1",
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "key_id": args.key_id,
            "public_key": public,
            "recovery_fingerprints": sorted(recipients),
            "storage": {
                "primary": "OWNER_LOCAL_ENCRYPTED",
                "backup": "REMOVABLE_MEDIA_ENCRYPTED",
                "separate_filesystems": True,
            },
            "envelope_sha256": {
                "primary": hashlib.sha256(primary_bytes).hexdigest(),
                "backup": hashlib.sha256(backup_bytes).hexdigest(),
            },
            "plaintext_key_persisted": False,
            "development_signing_custody": "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL",
            "product_signing_custody": "NOT_ESTABLISHED",
            "offline_revocation_freshness": "NOT_ESTABLISHED",
        }
        if record.exists() or record.is_symlink():
            raise ValueError("custody record already exists")
        record.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        publish(record, canonical(value) + b"\n")
        result = verify(primary, backup, record)
    except BaseException:
        for path in [primary, backup]:
            try:
                path.unlink()
            except FileNotFoundError:
                pass
        raise
    print(json.dumps(result, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create_parser = commands.add_parser("create")
    create_parser.add_argument("--key-id", required=True)
    create_parser.add_argument("--primary", type=Path, required=True)
    create_parser.add_argument("--backup", type=Path, required=True)
    create_parser.add_argument("--record", type=Path, required=True)
    create_parser.add_argument("--recipient", action="append", required=True)
    create_parser.add_argument("--forbid-root", action="append", type=Path, default=[])
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--primary", type=Path, required=True)
    verify_parser.add_argument("--backup", type=Path, required=True)
    verify_parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "create":
        create(args)
    else:
        print(
            json.dumps(verify(args.primary, args.backup, args.record), sort_keys=True)
        )


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        print(
            json.dumps({"result": "CUSTODY_REFUSED", "reason": str(error)}),
            file=sys.stderr,
        )
        raise SystemExit(2) from None
    except subprocess.CalledProcessError:
        print(
            json.dumps(
                {
                    "result": "CUSTODY_REFUSED",
                    "reason": "cryptographic command failed",
                }
            ),
            file=sys.stderr,
        )
        raise SystemExit(2) from None
