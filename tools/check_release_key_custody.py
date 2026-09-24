#!/usr/bin/env python3
"""Check that a specific local development private key is absent from committed trees.

Optional --image exports and inspects the actual flattened runtime filesystem.
No private bytes are printed or written into the report. This is a scoped custody
check, not a scan for every possible credential or a hardware signing ceremony.
"""
import argparse
import base64
import json
from pathlib import Path
import subprocess
import tarfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--key', required=True, type=Path)
    p.add_argument('--repo', action='append', required=True, type=Path)
    p.add_argument('--image')
    a = p.parse_args()
    raw = a.key.read_bytes()
    der = base64.b64decode(b''.join(line for line in raw.splitlines() if not line.startswith(b'-----')))
    patterns = [raw.strip(), der, der[-32:]]
    def contains(stream):
        tail = b''
        while True:
            data = stream.read(1024 * 1024)
            if not data:
                return False
            data = tail + data
            if any(pattern in data for pattern in patterns):
                return True
            tail = data[-max(map(len, patterns)):]
    counts = {}
    import io
    for repo in a.repo:
        paths = subprocess.check_output(['git', '-C', str(repo), 'ls-tree', '-r', '--name-only', 'HEAD', '-z']).split(b'\0')
        count = 0
        for name in filter(None, paths):
            data = subprocess.check_output(['git', '-C', str(repo), 'show', 'HEAD:' + name.decode()])
            if contains(io.BytesIO(data)):
                raise SystemExit('PRIVATE_KEY_FOUND_IN_COMMITTED_TREE')
            count += 1
        counts[str(repo)] = count
    files = 0
    if a.image:
        cid = subprocess.check_output(['docker', 'create', a.image], text=True).strip()
        try:
            process = subprocess.Popen(['docker', 'export', cid], stdout=subprocess.PIPE)
            with tarfile.open(fileobj=process.stdout, mode='r|') as archive:
                for member in archive:
                    if member.isfile():
                        files += 1
                        if contains(archive.extractfile(member)):
                            raise SystemExit('PRIVATE_KEY_FOUND_IN_RUNTIME_IMAGE')
            if process.wait() != 0:
                raise SystemExit('image export failed')
        finally:
            subprocess.run(['docker', 'rm', cid], stdout=subprocess.DEVNULL, check=True)
    print(json.dumps({'result': 'PRIVATE_KEY_ABSENT', 'committed_files': counts, 'runtime_files': files}))


if __name__ == '__main__':
    main()
