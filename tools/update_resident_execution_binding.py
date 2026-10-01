#!/usr/bin/env python3
"""Pin the optional resident execution binding and shared values."""
from pathlib import Path
import argparse, hashlib, json
root=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
sources=['proto/rx/resident/execution/v1/execution.proto','crates/rx-domain/src/resident_execution.rs','crates/rx-domain/src/component.rs','crates/rx-domain/src/component_transfer.rs','spec/resident-execution/v1/README.md']
value={'schema':'rx.resident-execution-binding.v1','package':'rx.resident.execution.v1','revision':1,'max_payload_bytes':262144,'source_sha256':{s:hashlib.sha256((root/s).read_bytes()).hexdigest() for s in sources}}
target=root/'spec/resident-execution/v1/binding.json'
if a.check:
 if json.loads(target.read_text())!=value:raise SystemExit('Resident execution binding source mismatch')
 print('Resident execution binding sources unchanged')
else:target.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
