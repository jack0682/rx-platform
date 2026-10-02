#!/usr/bin/env python3
"""Pin the explicit Executor v2 contract without changing v1."""
from pathlib import Path
import argparse
import hashlib
import json
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
files = [
    'proto/rx/executor/execution/v2/execution.proto',
    'crates/rx-process-contract/src/execution_v2.rs',
    'crates/rx-process-contract/src/execution_v2/executor.rs',
    'crates/rx-process-contract/src/execution_v2/materialize.rs',
    'crates/rx-process-contract/src/execution_v2/runtime_binding.rs',
    'spec/executor-execution/v2/README.md',
]
value = {'schema': 'rx.executor-execution-binding.v2', 'package': 'rx.executor.execution.v2',
         'max_payload_bytes': 1000000,
         'source_sha256': {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in files}}

path = root / 'spec/executor-execution/v2/binding.json'
if args.check:
    if json.loads(path.read_text()) != value:
        raise SystemExit('Executor execution v2 binding source mismatch')
    print('Executor execution v2 binding unchanged')
else:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
