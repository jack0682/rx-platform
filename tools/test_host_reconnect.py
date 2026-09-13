#!/usr/bin/env python3
"""Actual P-only restart with one idle product Host; no motion, commissioning or DB writes."""
from __future__ import annotations

import argparse
import hashlib
import json
import socket
import tempfile
import time
from pathlib import Path

from cell_delivery.api import Api, Rejected, publish_new
from cell_delivery.docker import Docker
from cell_delivery.installation import check_role_files, patch_s
from cell_delivery.materials import Materials

ROOT = Path(__file__).resolve().parents[1]

# A separate network-none reader opens the live SQLite WAL read-only. It selects
# only the fixture Host, its sessions, registrations and public cell records.
# Product processes remain the sole writers; this is an independent test oracle.
ORACLE = r'''
import hashlib,json,sqlite3,sys
from pathlib import Path
def key(kind,value):
    raw=json.dumps(value,sort_keys=True,separators=(',',':')).encode()
    return kind+'/'+hashlib.sha256(b'RX-ENTITY-KEY-v1\n'+raw).hexdigest()
uri='file:/data/platform/platform.db?mode=ro'
if sys.argv[1]=='stopped':
    wal=Path('/data/platform/platform.db-wal')
    assert not wal.exists() or wal.stat().st_size==0, 'stopped DB still has an uncheckpointed WAL'
    uri+='&immutable=1'
db=sqlite3.connect(uri,uri=True)
db.execute('PRAGMA query_only=ON')
db.execute('BEGIN')
def one(k):
    row=db.execute('SELECT revision,document FROM entities WHERE key=?',(k,)).fetchone()
    return None if row is None else {'revision':row[0],'value':json.loads(row[1])['value']}
producer=one(key('producer','host/sim'))
session=one(key('session',producer['value']['session'])) if producer else None
cursor=one(key('evidencecursor',['host/sim',producer['value']['journal']])) if producer else None
def selected(prefix,field,value):
    result=[]
    for k,revision,raw in db.execute('SELECT key,revision,document FROM entities WHERE key LIKE ?',(prefix+'/%',)):
        doc=json.loads(raw)['value']
        selected=doc
        for part in field.split('.'):selected=selected.get(part) if isinstance(selected,dict) else None
        if selected==value:result.append({'key':k,'revision':revision,'value':doc})
    return result
outbox=[]
for oid,state,raw in db.execute('SELECT id,state,document FROM outbox'):
    value=json.loads(raw)['value']
    if value.get('host')=='host/sim':outbox.append({'id':oid,'state':state,'value':value})
result={'producer':producer,'session':session,'evidence_cursor':cursor,'hosts':selected('host','id','host/sim'),
        'cells':selected('cell','configuration.id','cell/a'),
        'baselines':selected('host-binding-baseline','host','host/sim'),
        'plans':selected('host-link-plan','host','host/sim'),'outbox':outbox}
db.rollback();db.close()
print(json.dumps(result,sort_keys=True,separators=(',',':')))
'''


def until(check, accept, seconds=20):
    deadline=time.monotonic()+seconds
    while True:
        value=check()
        if accept(value):return value
        if time.monotonic()>=deadline:raise AssertionError('reconnection condition did not become true')
        time.sleep(.2)


def exercise(docker, materials, final, bundle, p_image, s_image, port, after_reconnect=None):
    browser=json.loads((final/'browser-fixture.json').read_text())
    delivery=json.loads((final/'delivery.json').read_text())
    check_role_files(final,browser)
    pc=docker.volume('p-config');pd=docker.volume('p-data');pw=docker.volume('p-work')
    imports=docker.volume('imports');ui=docker.volume('ui')
    docker.put(p_image['Id'],pc,final/'config');docker.put(p_image['Id'],imports,final/'import');docker.put(p_image['Id'],ui,bundle)
    docker.prepare_permissions(p_image['Id'],[pc+':/config',pd+':/data',pw+':/work'])
    mounts=[pc+':/config:ro',pd+':/data:rw',imports+':/import:ro',ui+':/operator:ro']
    docker.command(p_image['Id'],'/usr/local/bin/rx-platformd',['init','/config/startup.json'],mounts,'p-init')
    docker.make_network()
    def launch(label):
        return docker.start(p_image['Id'],label,'p','/usr/local/bin/rx-platformd',['run','/config/startup.json'],mounts,[f'127.0.0.1:{port}:8443'])
    def login(container,label):
        deadline=time.monotonic()+20
        while True:
            assert docker.state(container)['State']['Running'],'P exited during startup'
            try:
                api=Api(browser['origin'],final/browser['ca'],final/browser['certificate'],final/browser['private_key'],docker.evidence/'api'/label)
                api.login('installer',browser['credentials']['installer'])
                return api,api.get('/api/v1/overview')
            except (OSError,Rejected):
                if time.monotonic()>=deadline:raise
                time.sleep(.2)
    p=launch('p-original');api,overview=login(p,'original')
    installation=overview['installation']
    engine=materials.temporary/'rx-bt-engine';holder=docker.holder(s_image['Id'],[])
    docker.run('cp',holder+':/opt/rx/bin/rx-bt-engine',str(engine))
    patch_s(final,installation,hashlib.sha256(engine.read_bytes()).hexdigest())
    hc=docker.volume('h-config');hd=docker.volume('h-data');hr=docker.volume('h-runtime')
    docker.put(s_image['Id'],hc,final/'host-config')
    docker.prepare_permissions(s_image['Id'],[hc+':/config',hd+':/data',hr+':/work'])
    hm=[hc+':/config/host:ro',hd+':/data:rw',hr+':/run/rx-host:rw']
    docker.command(s_image['Id'],'/opt/rx/bin/rx-hostd',['init','/config/host/startup.json'],hm,'h-init')
    h=docker.start(s_image['Id'],'h','s','/opt/rx/bin/rx-hostd',['run','/config/host/startup.json'],hm)
    reads=0;host_reads=0
    def oracle(stopped=False):
        nonlocal reads
        reads+=1
        return json.loads(docker.command(s_image['Id'],'/usr/bin/python3',['-c',ORACLE,'stopped' if stopped else 'live'],[pd+':/data:ro'],f'oracle-{reads:03}'))
    def host_status():
        nonlocal host_reads
        assert docker.state(h)['State']['Running'],'Host must remain the same running process'
        value=json.loads(docker.run('exec',h,'cat','/run/rx-host/host-status.json'))
        host_reads+=1;publish_new(docker.evidence/f'host-status-{host_reads:03}.json',value)
        return value
    def evidence_count(label):
        script="import sqlite3; d=sqlite3.connect('file:/data/host/host.db?mode=ro',uri=True); d.execute('PRAGMA query_only=ON'); print(d.execute('SELECT COUNT(*) FROM events').fetchone()[0]); d.close()"
        return int(docker.command(s_image['Id'],'/usr/bin/python3',['-c',script],[hd+':/data:ro'],label))
    initial=until(oracle,lambda v:v['producer'] is not None and len(v['hosts'])==1)
    host_before=until(host_status,lambda value:'Idle' in value['publication'])
    assert evidence_count('host-evidence-before')==0
    descriptor_before=json.loads(docker.run('exec',h,'cat','/data/host/installation.json'))
    old_session=initial['producer']['value']['session']
    assert initial['hosts'][0]['value']['session']==old_session
    def stop_p(container):
        docker.run('kill','--signal','TERM',container)
        state=until(lambda:docker.state(container)['State'],lambda s:not s['Running'],30)
        assert state['ExitCode'] in (0,2),state
        return state['ExitCode']
    first_exit=stop_p(p)
    stopped=oracle(stopped=True)
    assert stopped['producer']['value']['session']==old_session
    docker.capture_logs()
    docker.run('network','disconnect',docker.network,p)
    docker.run('network','disconnect',docker.front_network,p)
    restarted=launch('p-restarted');new_api,new_overview=login(restarted,'restarted')
    new_installation=new_overview['installation']
    assert installation['id']==new_installation['id'] and installation['store_generation']==new_installation['store_generation']
    assert installation['clock_id']==new_installation['clock_id']
    assert installation['runtime_boot']!=new_installation['runtime_boot']
    current=until(oracle,lambda v:v['producer'] is not None and v['producer']['value']['session']!=old_session
                  and delivery['cell'] in v['producer']['value']['cells'])
    original=initial['producer']['value'];producer=current['producer']['value']
    for field in ['principal','peer_boot','journal','authentication_binding']:
        assert producer[field]==original[field],field
    assert current['session']['value']['active'] and current['session']['value']['runtime_boot']==new_installation['runtime_boot']
    assert current['hosts']==stopped['hosts'],'evidence reconnect must not rewrite operating registration'
    assert current['hosts'][0]['value']['session']==old_session
    # Repeated idle probes must reuse the new session instead of creating more.
    assert current['evidence_cursor'] is not None
    stable=until(oracle,lambda value:value['evidence_cursor'] is not None
                 and value['evidence_cursor']['revision']>=current['evidence_cursor']['revision']+2,10)
    assert stable['producer']['value']['session']==producer['session']
    assert stable['hosts']==stopped['hosts']
    assert current['evidence_cursor'] is not None and stable['evidence_cursor'] is not None
    assert stable['evidence_cursor']['revision']>=current['evidence_cursor']['revision']+2, 'two further empty batches must actually reach P'
    assert stable['evidence_cursor']['value']['through']==current['evidence_cursor']['value']['through']=='0'
    host_after=until(host_status,lambda value:'Idle' in value['publication'])
    for field in ['host_boot','instance','installation','clock_id']:
        assert host_after[field]==host_before[field],field
    assert evidence_count('host-evidence-after')==0
    assert json.loads(docker.run('exec',h,'cat','/data/host/installation.json'))==descriptor_before
    effects=docker.run('exec',h,'/bin/sh','-c','if [ -f /data/host/device/effects.jsonl ]; then cat /data/host/device/effects.jsonl; fi')
    assert not effects.strip(),'reconnection must not create a native effect'
    final_overview=new_api.get('/api/v1/overview')
    cell=next(c for c in final_overview['cells'] if c['cell']['value']['id']==delivery['cell'])
    assert not cell['runs'] and cell['cell']['value']['qualification'] is None
    before_blocks={b['id']:b for b in stopped['cells'][0]['value']['blocks']}
    after_blocks={b['id']:b for b in stable['cells'][0]['value']['blocks']}
    assert all(after_blocks.get(key)==value for key,value in before_blocks.items()),'old restrictions must remain intact'
    added=[b for key,b in after_blocks.items() if key not in before_blocks]
    assert added and all(b['reason']=='RUNTIME_RESTART' for b in added),added
    extension = None
    if after_reconnect is not None:
        extension = after_reconnect({
            'docker':docker,'materials':materials,'final':final,'browser':browser,'delivery':delivery,
            'p_image':p_image,'s_image':s_image,'port':port,'installation':new_installation,
            'initial':initial,'stopped':stopped,'stable':stable,'h':h,'p':restarted,
            'p_data':pd,'h_data':hd,'oracle':oracle,'host_status':host_status,
            'evidence_count':evidence_count,'api':new_api,
        })
    docker.run('kill','--signal','TERM',h)
    h_stop=until(lambda:docker.state(h)['State'],lambda state:not state['Running'],30)
    assert h_stop['ExitCode']==0
    final_exit=stop_p(restarted)
    publish_new(docker.evidence/'result.json',{
        'schema':'rx.idle-host-reconnect-test.v1','status':'PASS','simulation_only':True,
        'platform_image':p_image['Id'],'solutions_image':s_image['Id'],
        'old_installation':installation,'new_installation':new_installation,
        'before':initial,'after_p_stop':stopped,'after_reconnect':current,'after_repeated_probes':stable,
        'host_before':host_before,'host_after':host_after,'host_installation':descriptor_before,
        'native_effects':0,'new_evidence_records':0,'operating_registration_rebound':False,
        'qualification_restored':False,'run_count':0,'p_stop_exits':[first_exit,final_exit],
        'extension':extension,
        'oracle':'Independent read-only SQLite transaction; no direct writes or state injection.',
        'limitations':['Evidence/session communication only; operating rebind and restart/resume remain unimplemented.',
                       'Unqualified FILE_SIMULATION cell; no physical or commissioned recovery acceptance.']})


def main(after_reconnect=None):
    parser=argparse.ArgumentParser()
    parser.add_argument('--platform-image',default='rx-platform:runtime-draft')
    parser.add_argument('--solutions-image',default='rx-solutions:runtime-draft')
    parser.add_argument('--evidence-dir',type=Path,required=True)
    args=parser.parse_args();args.evidence_dir.mkdir(parents=True,exist_ok=False)
    docker=Docker(args.evidence_dir.resolve())
    try:
        with tempfile.TemporaryDirectory(prefix='rx-idle-reconnect-') as directory:
            temporary=Path(directory);p_image=docker.image(args.platform_image);s_image=docker.image(args.solutions_image)
            with socket.socket() as connection:connection.bind(('127.0.0.1',0));port=connection.getsockname()[1]
            bundle=temporary/'operator';holder=docker.holder(s_image['Id'],[])
            docker.run('cp',holder+':/opt/rx/operator',str(bundle))
            materials=Materials(ROOT,temporary,docker.evidence,docker,s_image['Id'])
            materials.create_seed(s_image['Architecture']);package,compiled,compiler=materials.compile()
            validator=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
            final=materials.finalize(package,compiled,compiler,port,bundle,validator)
            exercise(docker,materials,final,bundle,p_image,s_image,port,after_reconnect)
    finally:docker.cleanup()


if __name__=='__main__':main()
