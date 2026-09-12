#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
sources=['proto/rx/executor/production/v1/production.proto','crates/rx-process-contract/src/production.rs','crates/rx-process-contract/src/execution.rs','spec/production/v1/README.md']
value={'schema':'rx.production-binding.v1','package':'rx.executor.production.v1','view_schema':'rx.production-state.v1','max_payload_bytes':1000000,'source_sha256':{f:hashlib.sha256((root/f).read_bytes()).hexdigest() for f in sources}}
path=root/'spec/production/v1/binding.json'
if args.check:
    if json.loads(path.read_text())!=value:raise SystemExit('production binding source mismatch')
    print('Production binding sources unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
