#!/usr/bin/env python3
"""Pin the explicit optional read contract; --check verifies rather than rewriting it."""
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
sources=['proto/rx/executor/v1/executor.proto','crates/rx-process-contract/src/execution.rs','crates/rx-process-contract/src/execution_validation.rs','spec/executor/v1/README.md']
value={'schema':'rx.executor-read-binding.v1','package':'rx.executor.v1','snapshot_schema':'rx.execution-snapshot.v1','max_payload_bytes':1000000,
       'source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in sources}}
path=root/'spec/executor/v1/binding.json'
if args.check:
    if json.loads(path.read_text())!=value:raise SystemExit('executor read binding source mismatch')
    print('Executor read binding sources unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
