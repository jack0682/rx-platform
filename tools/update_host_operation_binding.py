#!/usr/bin/env python3
"""Pin the explicit operation transport, never modify v1."""
from pathlib import Path
import argparse, hashlib, json
root=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(); p.add_argument('--check',action='store_true'); a=p.parse_args()
files=['proto/rx/host/execution/v2/execution.proto','crates/rx-process-contract/src/execution_v2.rs','crates/rx-process-contract/src/execution_v2/operation.rs','spec/host-execution/v2/README.md']
value={'schema':'rx.host-execution-binding.v2','package':'rx.host.execution.v2','max_payload_bytes':1000000,'source_sha256':{f:hashlib.sha256((root/f).read_bytes()).hexdigest() for f in files}}
path=root/'spec/host-execution/v2/binding.json'
if a.check:
    if json.loads(path.read_text())!=value: raise SystemExit('Host execution binding mismatch')
    print('Host execution binding unchanged')
else: path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
