#!/usr/bin/env python3
"""Pin the separate execution v2 Host qualification contract without changing v1."""
from pathlib import Path
import argparse
import hashlib
import json
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
files = [
    'proto/rx/host/qualification/v2/qualification.proto',
    'crates/rx-process-contract/src/execution_v2.rs',
    'crates/rx-process-contract/src/execution_v2/host_inputs.rs',
    'crates/rx-process-contract/src/execution_v2/host_qualification.rs',
    'crates/rx-process-contract/src/execution_v2/materialize.rs',
    'crates/rx-process-contract/src/execution_v2/templates.rs',
    'crates/rx-domain/src/host_qualification.rs',
    'crates/rx-domain/src/host_configuration.rs',
    'crates/rx-process-contract/src/execution_v2/host_configuration.rs',
    'spec/host-qualification/v2/README.md',
]
value = {'schema': 'rx.host-execution-qualification-binding.v2',
         'package': 'rx.host.qualification.v2', 'max_payload_bytes': 1000000,
         'request_schema': 'rx.host-execution-qualification-request.v2',
         'observation_schema': 'rx.host-execution-qualification-observation.v2',
         'source_sha256': {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in files}}
path = root / 'spec/host-qualification/v2/binding.json'
if args.check:
    if json.loads(path.read_text()) != value:
        raise SystemExit('Host execution qualification v2 binding source mismatch')
    print('Host execution qualification v2 binding unchanged')
else:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
