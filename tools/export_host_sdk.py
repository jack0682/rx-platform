#!/usr/bin/env python3
"""Export an exact SDK inventory; exclude authority code and refuse modified destinations."""
from pathlib import Path
import argparse, hashlib, json, os, re, shutil, tempfile

parser=argparse.ArgumentParser()
parser.add_argument('destination',type=Path)
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
destination=args.destination.absolute()
members=['rx-domain','rx-ports','rx-storage','rx-protocol','rx-package','rx-process-contract']

def inventory(directory):
    result={}
    for path in sorted(directory.rglob('*')):
        if path.is_symlink(): raise SystemExit(f'SDK symlink refused: {path}')
        relative=str(path.relative_to(directory))
        if path.is_file() and relative!='source-lock.json':
            result[relative]=hashlib.sha256(path.read_bytes()).hexdigest()
    return result

if destination.is_symlink(): raise SystemExit('SDK destination must not be a symlink')
if destination.exists() and any(destination.iterdir()):
    lock=destination/'source-lock.json'
    if not lock.is_file(): raise SystemExit('Existing nonempty destination is not an exported SDK')
    expected=json.loads(lock.read_text())
    if expected.get('schema')!='rx.host-sdk.v1' or expected.get('files')!=inventory(destination):
        raise SystemExit('Existing SDK differs from its recorded inventory; review local changes before re-export')

def copy_checked(source,target):
    if source.is_symlink() or (source.is_dir() and any(p.is_symlink() for p in source.rglob('*'))):
        raise SystemExit(f'SDK source symlink refused: {source}')
    if source.is_dir(): shutil.copytree(source,target)
    elif source.is_file(): shutil.copyfile(source,target)

destination.parent.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix=destination.name+'.export-',dir=destination.parent) as temporary:
    scratch=Path(temporary);staged=scratch/'new';staged.mkdir()
    cargo=(root/'Cargo.toml').read_text()
    cargo=re.sub(r'members = \[[^]]+\]', 'members = '+json.dumps(['crates/'+m for m in members]),cargo,count=1)
    (staged/'Cargo.toml').write_text(cargo)
    for member in members:
        target=staged/'crates'/member;target.mkdir(parents=True)
        for entry in ['Cargo.toml','build.rs','src','migrations']:
            source=root/'crates'/member/entry
            if source.exists(): copy_checked(source,target/entry)
    for folder in ['proto','spec']:copy_checked(root/folder,staged/folder)
    files=inventory(staged)
    (staged/'source-lock.json').write_text(json.dumps({'schema':'rx.host-sdk.v1','files':files},indent=2,sort_keys=True)+'\n')
    previous=scratch/'previous'
    if destination.exists():os.replace(destination,previous)
    try:os.replace(staged,destination)
    except BaseException:
        if previous.exists():os.replace(previous,destination)
        raise
print(f'Exported {len(files)} SDK files; rx-application is excluded')
