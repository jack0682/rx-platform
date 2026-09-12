#!/usr/bin/env python3
"""Acceptance of shipped P/S images with scoped FILE_SIMULATION and real product clocks."""
from __future__ import annotations
import argparse
import hashlib
import json
import socket
import tempfile
from pathlib import Path
from cell_delivery.api import publish_new
from cell_delivery.docker import Docker
from cell_delivery.materials import Materials

ROOT=Path(__file__).resolve().parents[1]


def validator_identity() -> str:
    digest=hashlib.sha256(b'RX-FILE-SIMULATION-DELIVERY-ACCEPTANCE-v1\0')
    files=[Path(__file__),*sorted((ROOT/'tools/cell_delivery').glob('*.py'))]
    for path in files:
        digest.update(str(path.relative_to(ROOT)).encode()+b'\0')
        digest.update(path.read_bytes())
    return digest.hexdigest()


def main() -> None:
    parser=argparse.ArgumentParser()
    parser.add_argument('--platform-image',default='rx-platform:runtime-draft')
    parser.add_argument('--solutions-image',default='rx-solutions:runtime-draft')
    parser.add_argument('--evidence-dir',type=Path,required=True)
    parser.add_argument('--release-evidence',type=Path,default=ROOT.parent/'references/implementation/phase76_checks.json')
    parser.add_argument('--prepare-only',action='store_true',help='Validate offline material preparation without claiming runtime acceptance')
    parser.add_argument('--composition',choices=['independent','supervisor'],default='independent')
    args=parser.parse_args()
    args.evidence_dir.mkdir(parents=True,exist_ok=False)
    evidence=args.evidence_dir.resolve();docker=Docker(evidence)
    with tempfile.TemporaryDirectory(prefix='rx-cell-delivery-') as folder:
        temporary=Path(folder)
        p_image=docker.image(args.platform_image);s_image=docker.image(args.solutions_image)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
        try:
            bundle=temporary/'operator'
            holder=docker.holder(s_image['Id'],[])
            docker.run('cp',holder+':/opt/rx/operator',str(bundle))
            materials=Materials(ROOT,temporary,evidence,docker,s_image['Id'])
            materials.create_seed(s_image['Architecture'])
            package,compiled,compiler_id=materials.compile()
            validator=validator_identity()
            final=materials.finalize(package,compiled,compiler_id,port,bundle,validator)
            delivery=json.loads((final/'delivery.json').read_text())
            assert not delivery['qualification_report_generated']
            assert delivery['target_configuration_digest']==hashlib.sha256((final/'reference/target-cell.json').read_bytes()).hexdigest()
            publish_new(evidence/'preparation.json',{
                'schema':'rx.cell-delivery-materials.v1','status':'PASS','runtime_acceptance':False,
                'platform_image':p_image['Id'],'solutions_image':s_image['Id'],
                'compiler_validator':compiler_id,'qualification_validator':validator,
                'delivery':delivery,'source_private_keys_published':False,
            })
            if not args.prepare_only:
                from cell_delivery.installation import exercise
                exercise(docker,materials,final,bundle,p_image,s_image,port,validator,args.release_evidence.resolve(),args.composition)
        finally:
            docker.cleanup()


if __name__=='__main__':main()
