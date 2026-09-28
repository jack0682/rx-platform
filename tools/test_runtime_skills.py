#!/usr/bin/env python3
"""Actual installed skill CLI -> public P -> Executor -> Host FILE_SIMULATION.

Offline fixture generation/signing is test-only. No P database authority seed and
no product fault switch. A cold CLI is killed after a real StartRun reply but before
its journal records that reply; independent P reads and Host effects prove recovery.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import uuid
from cell_delivery.api import publish_new
from cell_delivery.docker import Docker
from cell_delivery.materials import Materials
from cell_delivery.installation import exercise
from cell_delivery.commission import wait_for

ROOT = Path(__file__).resolve().parents[1]


def recovery_evidence(docker, image, solutions, temporary):
    holder = docker.holder(image['Id'], [])
    directory = docker.evidence / 'recovery'; directory.mkdir()
    manifest = directory / 'source.sha256'
    docker.run('cp', holder + ':/opt/rx/bin/source.sha256', str(manifest))
    files = {}
    for line in manifest.read_text().splitlines():
        expected, path = line.split('  ', 1)
        relative = path.removeprefix('/source/')
        file = solutions / relative
        if relative == path or not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest() != expected:
            raise ValueError('selected S image source differs: ' + relative)
        files[relative] = expected
    logs = {}
    for name in ['host-recovery.log', 'executor-recovery.log', 'executor-identity.log']:
        dest = directory / name
        docker.run('cp', holder + ':/opt/rx/bin/' + name, str(dest))
        logs[name] = hashlib.sha256(dest.read_bytes()).hexdigest()
    client = temporary / 'installed-client'
    docker.run('cp', holder + ':/opt/rx/client', str(client))
    for name in ['rx','runtime_client.py','image_identity.py']:
        if (client/name).read_bytes() != (solutions/'deployment/local-skills'/name).read_bytes():
            raise ValueError('installed client differs from selected source: ' + name)
        files['deployment/local-skills/'+name] = hashlib.sha256((client/name).read_bytes()).hexdigest()
    archive = directory / 'source.tar'
    with tarfile.open(archive, 'w') as target:
        for relative in sorted(files): target.add(solutions/relative, arcname='rx-solutions/'+relative, recursive=False)
    record = {'status':'PASS_FOR_REPORTED_SCOPE','images':{'solutions':image['Id']},
        'archive':{'path':archive.name,'sha256':hashlib.sha256(archive.read_bytes()).hexdigest()},
        'logs_sha256':logs,'scope':'Current S generic recovery tests; actual runtime skill acceptance is evaluated separately.'}
    publish_new(directory/'release-evidence.json',record)
    return directory/'release-evidence.json', client


def author_installed_process(c, client, request_id):
    """Compare actual P-authored bytes and reassembly before any package review/activation."""
    private=c['materials'].temporary/'initial-author';private.mkdir(mode=0o700)
    browser=c['browser'];final=c['final'];d=c['docker']
    password=private/'password';password.write_text(browser['credentials']['engineer']);password.chmod(0o600)
    connection=private/'connection.json'
    connection.write_text(json.dumps({'schema':'rx.runtime-skill-connection.v1','origin':browser['origin'],
        'ca':str(final/browser['ca']),'certificate':str(final/browser['certificate']),
        'private_key':str(final/browser['private_key']),'principal':'engineer','password_file':str(password)}))
    connection.chmod(0o600)
    command=[sys.executable,str(client/'rx'),'runtime','--connection',str(connection),'--state-dir',str(private/'journal'),
        'compose','delivery/composed','--cell',c['delivery']['cell'],'--step','step/cycle','--step','step/cycle','--request-id',request_id]
    reply=subprocess.run(command,capture_output=True,text=True,timeout=100)
    if reply.returncode:raise RuntimeError('initial authoring failed: '+reply.stderr[-3000:])
    authored=json.loads(reply.stdout)
    expected=json.loads((c['materials'].seed/'compile-input.json').read_text())
    assert authored['compile_input']==expected and not authored['execution_authorized']
    public=private/'public';public.mkdir();publish_new(public/'online-compile-input.json',authored['compile_input'])
    d.put(c['s_image']['Id'],c['materials'].compiler_work,public)
    assembled=json.loads(d.command(c['s_image']['Id'],'/opt/rx/bin/rx-process-package',
        ['assemble','/work/online-compile-input.json','/config/package-recipe.json','/work/online-candidate'],
        [c['materials'].compiler_config+':/config:ro',c['materials'].compiler_work+':/work:rw'],'online-package-assemble'))
    assert assembled['manifest_digest']==c['delivery']['package']['manifest']
    c['installed_composition']=authored
    publish_new(d.evidence/'installed-composition.json',{'status':'PASS','authored':authored,'assembly':assembled,
        'scope':'Actual online P export and reassembled manifest equal the predeclared package before public review and activation; no live policy substitution'})


def invoke_scene(context, active):
    c = context; d = c['docker']; final = c['final']; browser = c['browser']
    client = c['installed_client']; private = c['materials'].temporary/'runtime-client'; private.mkdir(mode=0o700)
    password = private/'password'; password.write_text(browser['credentials']['operator']); password.chmod(0o600)
    connection = private/'connection.json'
    value = {'schema':'rx.runtime-skill-connection.v1','origin':browser['origin'],
        'ca':str(final/browser['ca']),'certificate':str(final/browser['certificate']),
        'private_key':str(final/browser['private_key']),'principal':'operator','password_file':str(password)}
    connection.write_text(json.dumps(value)); connection.chmod(0o600)
    state = private/'journal'; result_id = str(uuid.uuid4())
    base = [sys.executable,str(client/'rx'),'runtime','--connection',str(connection),'--state-dir',str(state)]
    def call(*args, expected=0):
        result = subprocess.run(base+list(args),capture_output=True,text=True,timeout=100)
        if result.returncode != expected:raise RuntimeError(f'installed runtime CLI failed: {result.returncode}: {result.stderr[-3000:]} {result.stdout[-1000:]}')
        return json.loads(result.stdout)
    catalog = call('skills')
    selected = next(v for v in catalog['bindings'] if v['binding']['cell']==c['delivery']['cell'])
    name = selected['binding']['name']; cell = selected['binding']['cell']
    # Compose using an Engineer session; this must not start a Run or arm hardware.
    engineer_password = private/'engineer-password'
    engineer_password.write_text(browser['credentials']['engineer']); engineer_password.chmod(0o600)
    engineer_connection = private/'engineer-connection.json'
    engineer_connection.write_text(json.dumps(dict(value,principal='engineer',password_file=str(engineer_password))))
    engineer_connection.chmod(0o600)
    author = [sys.executable,str(client/'rx'),'runtime','--connection',str(engineer_connection),'--state-dir',str(state)]
    def author_call(*args):
        reply=subprocess.run(author+list(args),capture_output=True,text=True,timeout=100)
        if reply.returncode:raise RuntimeError('installed authoring CLI failed: '+reply.stderr[-3000:])
        return json.loads(reply.stdout)
    options=author_call('steps','--cell',cell)
    step=options['candidates'][0]['step'];composition_id=str(uuid.uuid4())
    author_crash=private/'crash_author.py'
    author_crash.write_text("import os,runpy,signal,sys\nfrom pathlib import Path\nsys.path.insert(0,sys.argv[1])\nimport runtime_client\nreal=runtime_client.Terminal.request\ndef cut(self,path,body=None):\n value=real(self,path,body)\n if path=='/api/v1/process-draft-bindings':os.kill(os.getpid(),signal.SIGKILL)\n return value\nruntime_client.Terminal.request=cut\nscript=str(Path(sys.argv[1])/'rx')\nsys.argv=[script]+sys.argv[2:]\nrunpy.run_path(script,run_name='__main__')\n")
    author_args=['runtime','--connection',str(engineer_connection),'--state-dir',str(state),
        'compose','composed-transfer','--cell',cell,'--step',step,'--step',step,'--request-id',composition_id]
    author_cut=subprocess.run([sys.executable,str(author_crash),str(client),*author_args],capture_output=True,text=True,timeout=100)
    assert author_cut.returncode==-9,(author_cut.returncode,author_cut.stderr[-2000:])
    author_journal=state/composition_id
    assert not (author_journal/'compose-bindings.reply.json').exists()
    original_binding_request=(author_journal/'compose-bindings.request.json').read_bytes()
    composition=author_call('compose-recover',composition_id)
    assert (author_journal/'compose-bindings.request.json').read_bytes()==original_binding_request
    recovered_composition=author_call('compose-recover',composition_id)
    assert composition==recovered_composition and composition['execution_authorized'] is False
    source=composition['compile_input']['source']
    assert [n['body']['kind'] for n in source['flows'][0]['nodes']]==['SEQUENCE','OPERATION','OPERATION']
    assert len(composition['compile_input']['bindings'])==2
    authored=private/'authored';authored.mkdir()
    publish_new(authored/'compile-input.json',composition['compile_input'])
    d.put(c['s_image']['Id'],c['materials'].compiler_work,authored)
    assembled=d.command(c['s_image']['Id'],'/opt/rx/bin/rx-process-package',
        ['assemble','/work/compile-input.json','/config/package-recipe.json','/work/authored-candidate'],
        [c['materials'].compiler_config+':/config:ro',c['materials'].compiler_work+':/work:rw'],'authored-package-assemble')
    publish_new(d.evidence/'composition.json',{'status':'PASS','composition':composition,
        'assembly':json.loads(assembled),'consumer_exit':author_cut.returncode,'original_binding_request':json.loads(original_binding_request),'scope':'Actual P draft/binding registration and exported input accepted by existing package assembler; no activation of this new draft'})
    assert selected['commissioning']=='COMMISSIONED'
    assert selected['binding']['environment']=='SIMULATION'
    assert selected['binding']['input_mode']=='BOUND_CONFIGURATION'
    # External test instrumentation around the real transport. The unmodified
    # installed CLI performs login, public reads, CreateRun and StartRun.
    crash = private/'crash_client.py'
    crash.write_text("import os,runpy,signal,sys\nfrom pathlib import Path\nsys.path.insert(0,sys.argv[1])\nimport runtime_client\nreal=runtime_client.Terminal.request\ndef cut(self,path,body=None):\n value=real(self,path,body)\n if path=='/api/v1/runs/start':\n  os.kill(os.getpid(),signal.SIGKILL)\n return value\nruntime_client.Terminal.request=cut\nscript=str(Path(sys.argv[1])/'rx')\nsys.argv=[script]+sys.argv[2:]\nrunpy.run_path(script,run_name='__main__')\n")
    args = ['runtime','--connection',str(connection),'--state-dir',str(state),'run',name,'--cell',cell,'--count','1','--request-id',result_id]
    cut = subprocess.run([sys.executable,str(crash),str(client),*args],capture_output=True,text=True,timeout=100)
    assert cut.returncode == -9, (cut.returncode,cut.stderr[-2000:])
    journal = state/result_id
    original = (journal/'start.request.json').read_bytes()
    assert not (journal/'start.reply.json').exists()
    prepared = json.loads((journal/'prepare.reply.json').read_bytes())
    def native():
        raw=d.run('exec',c['h'],'/bin/sh','-c','if [ -f /data/host/device/effects.jsonl ]; then cat /data/host/device/effects.jsonl; fi')
        return [json.loads(line) for line in raw.splitlines() if line]
    expected_operations=len(c['target']['steps'])
    effects_before = wait_for(native,lambda rows:len(rows)==expected_operations,timeout=30)
    recovered = call('recover',result_id)
    assert (journal/'start.request.json').read_bytes()==original
    assert recovered['run_id']==prepared['id'] and recovered['result']['run']['value']['state']=='COMPLETED'
    assert recovered['result']['result_owner']=='PLATFORM' and not recovered['result']['details_truncated']
    assert len(recovered['result']['work'])==expected_operations
    assert recovered['result']['binding']['package_digest']==c['delivery']['package']['manifest']
    assert all(w['operation']['outcome']=='SUCCEEDED' and w['operation']['disposition']=='RELEASED' for w in recovered['result']['work'])
    assert all(p['value']['disposition']=='CONFIRMED_COMPLETED' for p in recovered['result']['parts'])
    repeated = call('recover',result_id)
    assert repeated['run_id']==recovered['run_id'] and len(native())==expected_operations
    next_id = str(uuid.uuid4())
    following = call('run',name,'--cell',cell,'--request-id',next_id,'--count','1')
    assert following['run_id']!=recovered['run_id'] and following['result']['run']['value']['state']=='COMPLETED'
    assert len(following['result']['work'])==expected_operations
    effects = native();assert len(effects)==2*expected_operations
    operations={w['operation']['operation_id'] for r in [recovered,following] for w in r['result']['work']}
    assert operations=={effect['operation'] for effect in effects}
    assert len({effect['invocation'] for effect in effects})==2*expected_operations
    publish_new(d.evidence/'runtime-skill-requests.json',{
        'original_start_request':json.loads(original),'recovered_start_reply':json.loads((journal/'start.reply.json').read_bytes()),
        'fault':'real consumer SIGKILL after P StartRun response and before durable receipt storage',
        'consumer_exit':cut.returncode,'native_effect_before_recovery':effects_before,'same_request_bytes_preserved':True})
    publish_new(d.evidence/'result.json',{'schema':'rx.runtime-skill-acceptance.v1','status':'PASS',
        'platform_image':c['p_image']['Id'],'solutions_image':c['s_image']['Id'],'catalog':catalog,
        'installed_draft':c['installed_composition']['draft_id'], 'operations_per_run':expected_operations,
        'original':recovered,'next':following,'independent_native_effects':effects,'physical_execution':'NOT_PERFORMED',
        'scope':'Online CLI-authored process, public review/activation and actual P/mTLS/Executor/Host FILE_SIMULATION execution, no P DB seed or local outcome authority',
        'limitations':['One approved bound process; no dynamic skill parameters or Python-to-Host registration in this slice.',
            'Consumer crash/receipt recovery is exercised; P or Host restart settlement is not claimed.',
            'Test-only external commissioning/signing material; no field qualification or one-command P/H/E installer claim.']})


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform-image',required=True)
    parser.add_argument('--solutions-image',required=True)
    parser.add_argument('--solutions-source',type=Path,required=True)
    parser.add_argument('--evidence',type=Path,required=True)
    args=parser.parse_args();args.evidence.mkdir(parents=True,exist_ok=False)
    evidence=args.evidence.resolve();d=Docker(evidence)
    with tempfile.TemporaryDirectory(prefix='rx-runtime-skills-') as temp:
        temporary=Path(temp)
        try:
            p=d.image(args.platform_image);s=d.image(args.solutions_image)
            recovery,client=recovery_evidence(d,s,args.solutions_source.resolve(),temporary)
            # Verify P image source too; mere image labels are insufficient.
            holder=d.holder(p['Id'],[]);manifest=temporary/'p-source.sha256'
            d.run('cp',holder+':/usr/local/bin/source.sha256',str(manifest))
            for line in manifest.read_text().splitlines():
                digest,path=line.split('  ',1);relative=path.removeprefix('/source/')
                assert relative!=path and hashlib.sha256((ROOT/relative).read_bytes()).hexdigest()==digest,relative
            composition_request=str(uuid.uuid4())
            composition_draft=str(uuid.uuid5(uuid.UUID(composition_request),'rx.runtime-skill.draft'))
            materials=Materials(ROOT,temporary,evidence,d,s['Id']);materials.create_seed(s['Architecture'],composition_draft)
            package,compiled,compiler=materials.compile()
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
            validator=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
            final=materials.finalize(package,compiled,compiler,port,None,validator)
            def after(c,active):
                c['installed_client']=client;invoke_scene(c,active)
            exercise(d,materials,final,None,p,s,port,validator,recovery,'independent',after_commissioning=after,before_commissioning=lambda c:author_installed_process(c,client,composition_request))
            print(json.dumps({'status':'PASS','evidence':str(evidence),'scope':'actual P/Host/Executor runtime skill invocation'}))
        finally:d.cleanup()


if __name__=='__main__':main()
