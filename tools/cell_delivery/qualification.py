"""External SIMULATION-only acceptance evidence assembly; never a P authority implementation."""
from __future__ import annotations
import hashlib
import json
import shutil
from pathlib import Path
from .api import encoded, publish_new

AREAS={'SOFTWARE','EQUIPMENT','CELL_INTEGRATION','RECOVERY','PROTECTION','OPERATIONS'}


def reference(value:dict) -> tuple[dict,bytes]:
    raw=encoded(value)
    return {'sha256':hashlib.sha256(raw).hexdigest(),'schema_id':value['schema'],'size_bytes':str(len(raw))},raw


def references(value:object) -> dict[str,dict]:
    result={}
    def walk(node):
        if isinstance(node,dict):
            if set(node)=={'sha256','schema_id','size_bytes'}:
                previous=result.setdefault(node['sha256'],node)
                if previous!=node:raise ValueError('one digest has conflicting artifact declarations')
            else:
                for child in node.values():walk(child)
        elif isinstance(node,list):
            for child in node:walk(child)
    walk(value)
    return result


def build_report(job:dict, validator:str, materials:Path, evidence_by_area:dict, output:Path)->dict:
    if set(evidence_by_area)!=AREAS:raise ValueError('all six independently measured areas required')
    if len(job['request']['cells'])!=1:raise ValueError('this acceptance harness covers one isolated simulation cell')
    target=job['request']['cells'][0];profile=target['profile']
    if profile['environment']!='SIMULATION':raise ValueError('delivery fixture signer cannot qualify physical cells')
    output.mkdir(exist_ok=False)
    artifacts=output/'artifacts';artifacts.mkdir()
    checks=[];generated={}
    for criterion in profile['criteria']:
        measured=evidence_by_area[criterion['area']]
        assertions=measured['assertions']
        if not assertions or any(value is not True for value in assertions.values()):
            raise ValueError(f"{criterion['area']} lacks actual passing assertions")
        body={'schema':criterion['evidence_schema'],'cell':profile['cell'],'criterion':criterion['id'],
              'scope':'FILE_SIMULATION_DELIVERY_ONLY','assertions':assertions,
              'observations':measured['observations'],'limitations':measured['limitations']}
        ref,raw=reference(body);generated[ref['sha256']]=raw
        checks.append({'cell':profile['cell'],'criterion':criterion['id'],'verdict':'PASS','evidence':[ref],
                       'note':f"Measured {criterion['area']} assertions for isolated software/file-device delivery only; no field qualification."})
    report={'schema':'rx.requalification-report.v1','request':job['request'],'validator':validator,'checks':checks}
    for digest,ref in references(report).items():
        raw=generated.get(digest)
        if raw is None:raw=(materials/'artifacts'/f'{digest}.bin').read_bytes()
        if hashlib.sha256(raw).hexdigest()!=digest or len(raw)!=int(ref['size_bytes']):
            raise ValueError('qualification artifact does not match its signed reference')
        (artifacts/f'{digest}.bin').write_bytes(raw)
    publish_new(output/'qualification.json',report)
    return report


def verified_release_evidence(path:Path, solutions_image:str)->dict:
    value=json.loads(path.read_text())
    if value['status']!='PASS_FOR_REPORTED_SCOPE' or value['images']['solutions']!=solutions_image:
        raise ValueError('recovery evidence must cover the exact selected solutions image')
    archive=value['archive'];source=path.parent/archive['path']
    if not source.resolve().is_relative_to(path.parent.resolve()):raise ValueError('local evidence archive required')
    if hashlib.sha256(source.read_bytes()).hexdigest()!=archive['sha256']:
        raise ValueError('sealed source archive changed')
    required=['sigkill_at_both_journal_native_boundaries_never_replays_device_effect',
              'lost_run_initialization_reply_recovers_binding_without_reinitializing',
              'run_creation_marker_cannot_change_after_header_initialization']
    witnesses={}
    for log,digest in value['logs_sha256'].items():
        file=path.parent/log
        if not file.resolve().is_relative_to(path.parent.resolve()):raise ValueError('local log required')
        raw=file.read_bytes()
        if hashlib.sha256(raw).hexdigest()!=digest:raise ValueError('sealed test log changed')
        content=raw.decode(errors='replace')
        for name in required:
            if f'test {name} ... ok' in content:witnesses[name]={'log':log,'sha256':digest}
    if set(witnesses)!=set(required):raise ValueError('specified recovery tests are not proven by release evidence')
    return {'evidence_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'image':solutions_image,
            'source_archive':archive,'verified_tests':witnesses,
            'scope':'exact release software recovery tests; not physical recovery certification'}
