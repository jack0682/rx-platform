#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
sources=['proto/rx/executor/assignment/v1/assignment.proto','crates/rx-process-contract/src/assignment.rs','crates/rx-process-contract/src/execution.rs','spec/assignment/v1/README.md']
value={'schema':'rx.assignment-binding.v1','package':'rx.executor.assignment.v1','view_schema':'rx.executor-assignment-state.v1','max_payload_bytes':65536,'source_sha256':{f:hashlib.sha256((root/f).read_bytes()).hexdigest() for f in sources}}
path=root/'spec/assignment/v1/binding.json'
if args.check:
    if json.loads(path.read_text())!=value:raise SystemExit('assignment binding source mismatch')
    print('Assignment binding sources unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
