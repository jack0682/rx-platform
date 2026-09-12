#!/usr/bin/env python3
"""Pin optional checkpoint preparation semantics without modifying frozen RX contracts."""
from pathlib import Path
import argparse, hashlib, json
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
sources = ['proto/rx/executor/plan/v1/plan.proto', 'crates/rx-protocol/src/checkpoint_rejection.rs', 'crates/rx-process-contract/src/checkpoint_change.rs', 'crates/rx-process-contract/src/execution.rs', 'spec/executor-plan/v1/README.md']
value = {'schema': 'rx.executor-plan-binding.v1', 'package': 'rx.executor.plan.v1', 'checkpoint_schema': 'rx.executor-state.v1', 'max_proposal_age_ns': '100000000', 'source_sha256': {f: hashlib.sha256((root/f).read_bytes()).hexdigest() for f in sources}}
path = root/'spec/executor-plan/v1/binding.json'
if args.check:
    if json.loads(path.read_text()) != value: raise SystemExit('executor plan binding source mismatch')
    print('Executor plan binding sources unchanged')
else:
    path.write_text(json.dumps(value, indent=2, sort_keys=True)+'\n')
