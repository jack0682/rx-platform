#!/usr/bin/env python3
"""Pin the optional resident reporting wire/value contract."""
from pathlib import Path
import argparse
import hashlib
import json

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
sources = ['proto/rx/resident/reporting/v1/reporting.proto',
           'crates/rx-domain/src/resident_reporting.rs',
           'crates/rx-domain/src/component.rs', 'spec/resident-reporting/v1/README.md']
value = {'schema': 'rx.resident-reporting-binding.v1', 'package': 'rx.resident.reporting.v1',
         'revision': 2, 'max_payload_bytes': 65536,
         'source_sha256': {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in sources}}
target = root / 'spec/resident-reporting/v1/binding.json'
if args.check:
    if json.loads(target.read_text()) != value:
        raise SystemExit('Resident reporting binding source mismatch')
    print('Resident reporting binding sources unchanged')
else:
    target.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
