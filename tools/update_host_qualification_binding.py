#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
files=['proto/rx/host/qualification/v1/qualification.proto','crates/rx-domain/src/host_qualification.rs','spec/host-qualification/v1/README.md']
value={'schema':'rx.host-qualification-binding.v1','package':'rx.host.qualification.v1','request_schema':'rx.host-qualification-request.v1','observation_schema':'rx.host-qualification-observation.v1','max_payload_bytes':1000000,'source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in files}}
path=root/'spec/host-qualification/v1/binding.json'
if a.check:
 if json.loads(path.read_text())!=value:raise SystemExit('Host qualification binding source mismatch')
 print('Host qualification binding unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
