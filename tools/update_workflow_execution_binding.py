#!/usr/bin/env python3
"""Pin the explicit v2 data contract; this does not enable a transport binding."""
from pathlib import Path
import argparse
import hashlib
import json

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
sources = [
    'crates/rx-process-contract/src/execution_v2.rs',
    'crates/rx-process-contract/src/execution_v2/materialize.rs',
    'crates/rx-process-contract/src/execution_v2/templates.rs',
    'spec/workflow-execution/v2/README.md',
]
value = {
    'schema': 'rx.workflow-execution-data-binding.v2',
    'transport': 'NOT_ENABLED',
    'policy_schema': 'rx.execution-policy.v2',
    'index_schema': 'rx.execution-report-index.v2',
    'report_schema': 'rx.execution-report.v2',
    'source_sha256': {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in sources},
}
path = root / 'spec/workflow-execution/v2/binding.json'
if args.check:
    if json.loads(path.read_text()) != value:
        raise SystemExit('Workflow execution v2 data binding mismatch')
    print('Workflow execution v2 data binding sources unchanged')
else:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
