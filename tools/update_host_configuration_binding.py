#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json
root=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
files=['proto/rx/host/configuration/v1/configuration.proto','crates/rx-domain/src/host_configuration.rs','spec/host-configuration/v1/README.md']
value={'schema':'rx.host-configuration-binding.v1','package':'rx.host.configuration.v1','request_schema':'rx.host-process-configuration-request.v1','observation_schema':'rx.host-process-configuration-observation.v1','max_payload_bytes':1000000,'source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in files}}
path=root/'spec/host-configuration/v1/binding.json'
if a.check:
 if json.loads(path.read_text())!=value:raise SystemExit('Host configuration binding source mismatch')
 print('Host configuration binding unchanged')
else:path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
