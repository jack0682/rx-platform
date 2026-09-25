#!/usr/bin/env python3
"""Build/install both client packages and compare the fixed corpus; no runtime mock."""
from pathlib import Path
import argparse
import json
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--work',type=Path,required=True);a=p.parse_args();work=a.work.absolute();work.mkdir(parents=True,exist_ok=False)
    records=[]
    def run(name,args):
        with (work/(name+'.stdout')).open('w') as out,(work/(name+'.stderr')).open('w') as err:
            result=subprocess.run(args,stdout=out,stderr=err)
        records.append({'name':name,'argv':list(map(str,args)),'exit_code':result.returncode});(work/'commands.json').write_text(json.dumps(records,indent=2))
        if result.returncode:raise RuntimeError(name+' failed; raw output retained')
    run('export',[sys.executable,ROOT/'tools/export_protocol.py',work/'bundle'])
    run('prepare',[sys.executable,ROOT/'tools/prepare_clients.py','--bundle',work/'bundle','--output',work/'prepared'])
    run('cpp-configure',['cmake','-S',work/'prepared/rxclcpp','-B',work/'cpp-build','-DCMAKE_BUILD_TYPE=Release','-DCMAKE_INSTALL_PREFIX='+str(work/'install')])
    run('cpp-build',['cmake','--build',work/'cpp-build','-j2'])
    run('cpp-corpus',[work/'cpp-build/rxclcpp-conformance',work/'bundle/strict-wire-v1/vectors.json'])
    run('cpp-install',['cmake','--install',work/'cpp-build'])
    run('python-wheel',[sys.executable,'-m','build','--wheel','--no-isolation','--outdir',work/'wheels',work/'prepared/rxclpy'])
    wheel=next((work/'wheels').glob('*.whl'))
    run('python-install',[sys.executable,'-m','pip','install','--no-index','--no-deps','--force-reinstall',wheel])
    run('python-corpus',[sys.executable,'-m','rxclpy.conformance'])
    run('consumer-configure',['cmake','-S',ROOT/'clients/examples/cpp','-B',work/'consumer','-DCMAKE_PREFIX_PATH='+str(work/'install'),'-DCMAKE_BUILD_TYPE=Release'])
    run('consumer-build',['cmake','--build',work/'consumer','-j2'])
    left=json.loads((work/'cpp-corpus.stdout').read_text())['cases'];right=json.loads((work/'python-corpus.stdout').read_text())['cases'];assert left==right
    result={'status':'CLIENT_PACKAGES_PASS','cases':len(left),'same_expanded_bytes_statuses_roundtrips':True,'bundle_sha256':json.loads((work/'prepared/build.json').read_text())['bundle_sha256']}
    (work/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
if __name__=='__main__':main()
