#!/usr/bin/env python3
"""Check that a reviewed development private key is absent from publication.

Optional --image exports and inspects the actual flattened runtime filesystem.
Committed history, current publishable files and optional image bytes are scanned.
No private bytes are printed or written into the report. This is a scoped custody
check, not a scan for every possible credential or a hardware signing ceremony.
"""
import argparse
import base64
import io
import json
from pathlib import Path
import subprocess
import tarfile
from development_release_custody import decrypt_envelope, public_key, validate_envelope


def git_blobs(repo):
    objects = subprocess.check_output(
        ["git", "-C", str(repo), "rev-list", "--objects", "--all"], text=True
    )
    identifiers = sorted({line.split()[0] for line in objects.splitlines() if line})
    checked = subprocess.run(
        ["git", "-C", str(repo), "cat-file", "--batch-check"],
        input="".join(identifier + "\n" for identifier in identifiers).encode(),
        stdout=subprocess.PIPE,
        check=True,
    )
    blobs = [
        line.split()[0].decode()
        for line in checked.stdout.splitlines()
        if len(line.split()) == 3 and line.split()[1] == b"blob"
    ]
    batch = subprocess.run(
        ["git", "-C", str(repo), "cat-file", "--batch"],
        input="".join(identifier + "\n" for identifier in blobs).encode(),
        stdout=subprocess.PIPE,
        check=True,
    )
    stream = io.BytesIO(batch.stdout)
    for _ in blobs:
        header = stream.readline().decode().strip().split()
        if len(header) != 3:
            raise SystemExit("git object stream malformed")
        _, kind, size = header
        data = stream.read(int(size))
        if stream.read(1) != b"\n":
            raise SystemExit("git object stream boundary")
        if kind == "blob":
            yield data


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--encrypted-key", required=True, type=Path)
    p.add_argument("--custody-record", required=True, type=Path)
    p.add_argument("--repo", action="append", required=True, type=Path)
    p.add_argument("--image")
    p.add_argument("--self-test", action="store_true")
    a = p.parse_args()
    validate_envelope(a.encrypted_key)
    custody = json.loads(a.custody_record.read_text())
    raw = decrypt_envelope(a.encrypted_key)
    if custody.get("schema") != "rx.development-release-custody.v1" or public_key(
        raw
    ).hex() != custody.get("public_key"):
        raise SystemExit("CUSTODY_RECORD_OR_KEY_MISMATCH")
    der = base64.b64decode(
        b"".join(line for line in raw.splitlines() if not line.startswith(b"-----"))
    )
    patterns = [raw.strip(), der, der[-32:]]

    def contains(stream):
        tail = b""
        while True:
            data = stream.read(1024 * 1024)
            if not data:
                return False
            data = tail + data
            if any(pattern in data for pattern in patterns):
                return True
            tail = data[-max(map(len, patterns)) :]

    if a.self_test and not contains(io.BytesIO(raw)):
        raise SystemExit("PRIVATE_KEY_SCANNER_NEGATIVE_CONTROL_FAILED")
    counts = {}
    for repo in a.repo:
        history_count = 0
        for data in git_blobs(repo):
            if contains(io.BytesIO(data)):
                raise SystemExit("PRIVATE_KEY_FOUND_IN_COMMITTED_HISTORY")
            history_count += 1
        paths = subprocess.check_output(
            [
                "git",
                "-C",
                str(repo),
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ]
        ).split(b"\0")
        current_count = 0
        for name in filter(None, paths):
            path = repo / name.decode()
            if path.is_file() and not path.is_symlink():
                with path.open("rb") as current:
                    if contains(current):
                        raise SystemExit("PRIVATE_KEY_FOUND_IN_CURRENT_TREE")
                current_count += 1
        counts[str(repo)] = {
            "history_blobs": history_count,
            "current_files": current_count,
        }
    files = 0
    if a.image:
        cid = subprocess.check_output(["docker", "create", a.image], text=True).strip()
        try:
            process = subprocess.Popen(
                ["docker", "export", cid], stdout=subprocess.PIPE
            )
            with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
                for member in archive:
                    if member.isfile():
                        files += 1
                        if contains(archive.extractfile(member)):
                            raise SystemExit("PRIVATE_KEY_FOUND_IN_RUNTIME_IMAGE")
            if process.wait() != 0:
                raise SystemExit("image export failed")
        finally:
            subprocess.run(["docker", "rm", cid], stdout=subprocess.DEVNULL, check=True)
    print(
        json.dumps(
            {
                "result": "PRIVATE_KEY_ABSENT",
                "repositories": counts,
                "runtime_files": files,
                "negative_control_detected": a.self_test,
                "private_bytes_emitted": False,
            }
        )
    )


if __name__ == "__main__":
    main()
