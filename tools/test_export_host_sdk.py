#!/usr/bin/env python3
"""Generated SDK export regression checks; all mutations stay in an owned temporary directory."""
from pathlib import Path
import hashlib,json,subprocess,sys,tempfile
root=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='rx-sdk-export-test-') as temporary:
    destination=Path(temporary)/'sdk'
    command=[sys.executable,str(root/'tools/export_host_sdk.py'),str(destination)]
    subprocess.run(command,check=True,capture_output=True)
    marker=destination/'old-generated-file.rs';marker.write_text('// obsolete generated source\n')
    lock=destination/'source-lock.json';metadata=json.loads(lock.read_text())
    metadata['files'][marker.name]=hashlib.sha256(marker.read_bytes()).hexdigest();lock.write_text(json.dumps(metadata))
    subprocess.run(command,check=True,capture_output=True)
    assert not marker.exists(), 'obsolete inventory member was carried into the new export'
    source=destination/'crates/rx-domain/src/lib.rs';source.write_text(source.read_text()+'\n// local modification\n');before=source.read_bytes()
    result=subprocess.run(command,capture_output=True,text=True)
    assert result.returncode!=0 and 'differs' in result.stderr
    assert source.read_bytes()==before, 'modified destination was overwritten'
    assert not (destination/'crates/rx-application').exists()
print('PASS: fresh inventory, modified destination preserved/refused, authority crate excluded')
