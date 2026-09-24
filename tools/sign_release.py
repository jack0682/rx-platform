#!/usr/bin/env python3
"""Offline development-release signing. Private key must never enter an image or Git.

The runtime has no signing/key input. Requires OpenSSL with Ed25519 support.
This authoring tool does not establish product release custody or qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

KEY_ID = 'rx/development-release-2026-09'
CHANNEL = 'rx/solutions-development'


def canonical(value):
    # This schema has only ASCII identifiers, hex digests and decimal strings;
    # sorted compact JSON equals the Rust JCS bytes for this restricted domain.
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def signed(key, domain, value):
    message = domain.encode() + b'\0' + KEY_ID.encode() + b'\0' + canonical(value)
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / 'message'
        path.write_bytes(message)
        signature = subprocess.check_output(['openssl', 'pkeyutl', '-sign', '-rawin', '-inkey', str(key), '-in', str(path)])
    return {'key': KEY_ID, 'signature': signature.hex()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--key', required=True, type=Path)
    p.add_argument('--inventory', required=True, type=Path)
    p.add_argument('--version', required=True, type=int)
    p.add_argument('--revocation-version', required=True, type=int)
    p.add_argument('--revoke', action='append', default=[])
    p.add_argument('--output', required=True, type=Path)
    a = p.parse_args()
    if not (0 < a.version < 2**64 and 0 < a.revocation_version < 2**64):
        p.error('versions must be positive u64 values')
    if any(len(d) != 64 or any(c not in '0123456789abcdef' for c in d) for d in a.revoke):
        p.error('revoked identities must be lowercase SHA256 digests')
    if a.key.is_symlink() or not a.key.is_file() or a.key.stat().st_mode & 0o077:
        p.error('key must be an owner-only regular file')
    manifest = {'schema': 'rx.release.v1', 'channel': CHANNEL, 'version': str(a.version),
                'inventory_sha256': hashlib.sha256(a.inventory.read_bytes()).hexdigest()}
    revocations = {'schema': 'rx.release-revocations.v1', 'channel': CHANNEL,
                   'version': str(a.revocation_version), 'revoked': sorted(set(a.revoke))}
    a.output.mkdir(parents=True, exist_ok=True)
    for filename, value, field, domain in [('release.json', manifest, 'manifest', 'RX-RELEASE-v1'),
                                           ('revocations.json', revocations, 'revocations', 'RX-RELEASE-REVOCATIONS-v1')]:
        output = a.output / filename
        if output.exists():
            p.error(f'output already exists: {output}')
        output.write_bytes(canonical({field: value, 'signature': signed(a.key, domain, value)}) + b'\n')
    print(json.dumps({'release_digest': hashlib.sha256(b'RX-RELEASE-IDENTITY-v1\n' + canonical(manifest)).hexdigest(),
                      'authority': 'DEVELOPMENT_ONLY; PRODUCT_CUSTODY_NOT_ESTABLISHED'}))


if __name__ == '__main__':
    main()
