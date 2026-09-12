#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
files=['proto/rx/host/read/v1/read.proto','crates/rx-domain/src/host_snapshot.rs','spec/host-read/v1/README.md']
value={'schema':'rx.host-read-binding.v1','package':'rx.host.read.v1','snapshot_schema':'rx.host-snapshot.v1','max_payload_bytes':1000000,'source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in files}}
path=root/'spec/host-read/v1/binding.json'
if a.check:
    if json.loads(path.read_text())!=value:raise SystemExit('Host read binding source mismatch')
    print('Host read binding unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
