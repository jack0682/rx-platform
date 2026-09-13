"""Shipped-image installation and HTTP commissioning acceptance; FILE_SIMULATION only."""
from __future__ import annotations
import hashlib
import json
import shutil
import time
from pathlib import Path
from .api import Api, Rejected, encoded, publish_new
from .commission import Commission, wait_for
from .qualification import build_report


def check_role_files(final:Path, browser:dict)->None:
    for role in ['P','H','E']:
        descriptor=browser['allowed_mounts'][role]
        root=final/descriptor['source']
        keys={str(p.relative_to(root)) for p in root.rglob('*.key')}
        if keys!=set(descriptor['private_keys']):raise ValueError(f'{role} has unexpected private keys')
        if list(root.rglob('signing-fixtures.json')):raise ValueError('signing seed leaked into runtime configuration')


def patch_s(final:Path, installation:dict, engine_sha:str)->None:
    data=json.loads((final/'host-config/binding-input.json').read_text())
    initial=data['initial_cell'];target=data['target_cell']
    assert initial['environment']==target['environment']=='SIMULATION'
    assert initial['definition']==target['definition'] and initial['envelope']==target['envelope']
    binding={'host':data['host'],'platform':data['platform'],'cell':initial['id'],
             'definition':initial['definition'],'envelope':initial['envelope'],
             'qualification':data['initial_unqualified_reference'],'qualification_revision':'1',
             'allowed_intents':[step['intent'] for step in initial['steps']],
             'scope_ids':initial['scopes'],'condition_ids':sorted({v for step in initial['steps'] for v in step['condition_ids']}),
             'environment':'SIMULATION','purposes':['PRODUCTION']}
    publish_new(final/'host-config/bindings.json',[binding])
    host=json.loads((final/'host-config/startup.template.json').read_text())
    assert host['backend']['kind']=='FILE_SIMULATION'
    host['publisher']['store_generation']=installation['store_generation']
    host['bindings']['sha256']=hashlib.sha256((final/'host-config/bindings.json').read_bytes()).hexdigest()
    publish_new(final/'host-config/startup.json',host)
    executor=json.loads((final/'executor-config/cell.template.json').read_text())
    executor['expected_service']['scope']['store_generation']=installation['store_generation']
    executor['engine']['sha256']=engine_sha
    publish_new(final/'executor-config/cell.json',executor)


def exercise(docker,materials,final:Path,bundle:Path,p_image:dict,s_image:dict,port:int,validator:str,release_evidence:Path,composition:str,*,start_services=None,after_commissioning=None)->None:
    browser=json.loads((final/'browser-fixture.json').read_text())
    delivery=json.loads((final/'delivery.json').read_text())
    target=json.loads((final/'reference/target-cell.json').read_text())
    check_role_files(final,browser)
    p_config=docker.volume('p-config');p_data=docker.volume('p-data');p_work=docker.volume('p-work')
    imports=docker.volume('imports');ui=docker.volume('ui')
    docker.put(p_image['Id'],p_config,final/'config')
    docker.put(p_image['Id'],imports,final/'import')
    docker.put(p_image['Id'],ui,bundle)
    docker.prepare_permissions(p_image['Id'],[p_config+':/config',p_data+':/data',p_work+':/work'])
    # Public import contents need readable ownership for later append-only publications too.
    docker.prepare_permissions(p_image['Id'],[imports+':/config',p_data+':/data',p_work+':/work'])
    mounts=[p_config+':/config:ro',p_data+':/data:rw',imports+':/import:ro',ui+':/operator:ro']
    docker.command(p_image['Id'],'/usr/local/bin/rx-platformd',['init','/config/startup.json'],mounts,'p-init')
    docker.make_network()
    p=docker.start(p_image['Id'],'p','p','/usr/local/bin/rx-platformd',['run','/config/startup.json'],mounts,[f'127.0.0.1:{port}:8443'])
    def api(who:str)->Api:
        value=Api(browser['origin'],final/browser['ca'],final/browser['certificate'],final/browser['private_key'],docker.evidence/'api'/who)
        value.login(who,browser['credentials'][who]);return value
    deadline=time.monotonic()+30
    while True:
        if not docker.state(p)['State']['Running']:raise RuntimeError('product P exited during startup')
        try:
            installer=api('installer');break
        except (OSError,Rejected):
            if time.monotonic()>deadline:raise
            time.sleep(.2)
    overview=installer.get('/api/v1/overview')
    installation=overview['installation']
    assert installation['id']==delivery['installation']
    assert installation['clock_id'].startswith('linux-boottime/')
    assert all(not cell['runs'] for cell in overview['cells'])
    engine=materials.temporary/'rx-bt-engine'
    source=docker.holder(s_image['Id'],[])
    docker.run('cp',source+':/opt/rx/bin/rx-bt-engine',str(engine))
    patch_s(final,installation,hashlib.sha256(engine.read_bytes()).hexdigest())
    context={'docker':docker,'materials':materials,'final':final,'browser':browser,'delivery':delivery,
             'target':target,'installation':installation,'p':p,'p_image':p_image,'s_image':s_image,
             'imports':imports,'validator':validator,'release_evidence':release_evidence,'composition':composition,
             'p_mounts':mounts,'p_data':p_data,'port':port,'after_commissioning':after_commissioning}
    if start_services is not None:
        if composition != 'independent':raise ValueError('fixture services require independent test composition')
        context.update(start_services(context))
    elif composition == 'supervisor':
        from .supervisor import start
        context.update(start(context))
    else:
        context.update(start_independent(docker,final,s_image))
    context['users']={who:api(who) for who in ['engineer','verifier','release','operator']}
    try:
        run_commissioning(context)
    finally:
        if composition == 'supervisor':
            from .supervisor import capture
            capture(context)


def start_independent(docker,final:Path,s_image:dict)->dict:
    # Same solutions image, isolated Host and Executor instances. No source/controller mounts.
    h_config=docker.volume('h-config');h_data=docker.volume('h-data');h_runtime=docker.volume('h-runtime')
    docker.put(s_image['Id'],h_config,final/'host-config')
    docker.prepare_permissions(s_image['Id'],[h_config+':/config',h_data+':/data',h_runtime+':/work'])
    h_mounts=[h_config+':/config/host:ro',h_data+':/data:rw',h_runtime+':/run/rx-host:rw']
    docker.command(s_image['Id'],'/opt/rx/bin/rx-hostd',['init','/config/host/startup.json'],h_mounts,'h-init')
    h=docker.start(s_image['Id'],'h','s','/opt/rx/bin/rx-hostd',['run','/config/host/startup.json'],h_mounts)
    e_config=docker.volume('e-config');e_data=docker.volume('e-data');e_work=docker.volume('e-work')
    docker.put(s_image['Id'],e_config,final/'executor-config')
    docker.prepare_permissions(s_image['Id'],[e_config+':/config',e_data+':/data',e_work+':/work'])
    e_mounts=[e_config+':/config/executor:ro',e_data+':/data:rw']
    docker.command(s_image['Id'],'/opt/rx/bin/rx-executor-service',['cell','init','/config/executor/cell.json'],e_mounts,'e-init')
    e=docker.start(s_image['Id'],'e','e','/opt/rx/bin/rx-executor-service',['cell','run','/config/executor/cell.json'],e_mounts)
    return {'h':h,'e':e,'h_data':h_data,'e_data':e_data}


def run_commissioning(context:dict)->None:
    # Filled by the actual public API workflow below; no bootstrap row or authority mutation.
    from .workflow import run
    run(context)
