#!/usr/bin/env python3
"""Installed C++/Python peers against actual P/H and public commissioning gates.

Explicit SIMULATION backend only. No private DB seed, client-owned authority,
new runtime entrypoint or claim of completed G5/ROBOTIS integration.
"""
from pathlib import Path
import argparse
import base64
import hashlib
import json
import os
import select
import shutil
import socket
import subprocess
import tempfile
import time
import uuid
from cell_delivery.api import publish_new, encoded, Api
from cell_delivery.docker import Docker
from cell_delivery.materials import Materials
from cell_delivery.installation import exercise
from cell_delivery.commission import wait_for

ROOT=Path(__file__).resolve().parents[1]
def uid():return str(uuid.uuid4())
def b64(value):return base64.b64encode(bytes.fromhex(value)).decode()

class Peer:
    def __init__(self,docker,context,language,config,label):
        self.docker=docker;self.serial=0;self.label=label
        volume=docker.volume(label+'-config');data=docker.volume(label+'-data');work=docker.volume(label+'-work')
        docker.put(context['client_image'],volume,config)
        docker.prepare_permissions(context['client_image'],[volume+':/config',data+':/data',work+':/work'])
        name=docker.prefix+'-'+label
        command=['docker','create','-i','--name',name,'--network',docker.network,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user','10001:10001','--tmpfs','/tmp:rw,uid=10001,gid=10001','-v',volume+':/config/executor:ro']
        program='/opt/client-env/bin/python' if language=='python' else '/opt/rx-client/bin/rxclcpp-peer'
        args=['/opt/rx-client/bin/python_peer.py','/config/executor/client.json'] if language=='python' else ['/config/executor/client.json']
        subprocess.run(command+['--entrypoint',program,context['client_image'],*args],check=True,capture_output=True)
        docker.containers.append(name);self.name=name
        self.stderr=(docker.evidence/(label+'-stderr.log')).open('w')
        self.process=subprocess.Popen(['docker','start','-ai',name],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.stderr,text=True,bufsize=1)
    def send(self,command):
        self.serial+=1;command={'id':self.serial,**command}
        publish_new(self.docker.evidence/f'{self.label}-{self.serial:03}-request.json',command)
        self.process.stdin.write(json.dumps(command)+'\n');self.process.stdin.flush();return command
    def receive(self,command,timeout=30):
        ready,_,_=select.select([self.process.stdout],[],[],timeout)
        if not ready:raise RuntimeError('peer reply timeout; no result inferred')
        line=self.process.stdout.readline()
        if not line:raise RuntimeError('peer exited before result; inspect stderr')
        response=json.loads(line);assert response['id']==command['id'],response
        publish_new(self.docker.evidence/f'{self.label}-{self.serial:03}-response.json',response);return response
    def call(self,method,request,ok=True,timeout=10):
        c=self.send({'method':method,'request':request,'timeout':timeout});v=self.receive(c,timeout+15)
        if ok:assert v['ok'],v
        return v['response'] if ok else v
    def action(self,action):return self.receive(self.send({'action':action}))
    def close(self):
        if self.process.poll() is None:
            self.action('close');self.process.stdin.close();self.process.wait(timeout=15)
        self.stderr.close()

class Scenario:
    def __init__(self,docker,language,mode,client_image):
        self.docker=docker;self.language=language;self.mode=mode;self.client_image=client_image;self.peer=None;self.c=None;self.adapter=bool(os.environ.get("RX_CELL_ADAPTER_DESCRIPTOR"))
    def context(self,key=None,revision=None):
        value={'session_id':self.session,'call_id':uid()}
        if key:value['request_key']=key
        if revision is not None:value['expected_revision']=str(revision)
        return value
    def negotiate(self,peer=None,boot=None):
        peer=peer or self.peer
        hello={**self.hello,'boot_id':boot or self.hello['boot_id']}
        session=peer.call('rx.contract.v1.SessionService/Open',hello)
        peer.call('rx.cell.v1.CellService/Open',{'base_session_id':session['session_id'],'peer_id':hello['peer_id'],'base_manifest_hash':self.base_hash,'cell_manifest_hash':self.cell_hash,'cell_definition_digest':self.definition,'shared_clock_id':hello['shared_clock_id']})
        return session['session_id']
    def start_services(self,c):
        self.c=c;c['client_image']=self.client_image;d=self.docker;s=c['s_image']['Id'];final=c['final']
        if self.adapter:
            c['observe_native_effects']=self.native_entries
            c['native_scope']='DYNAMIXEL_PROTOCOL2_PING_SIMULATED_TRANSPORT_ONLY'
            c['host_fixture_limitations']=['Fictional RX DYNAMIXEL model65500; protocol2 Ping; no physical qualification']
        hc=d.volume('h-config');hd=d.volume('h-data');hr=d.volume('h-runtime')
        d.put(s,hc,final/'host-config');d.prepare_permissions(s,[hc+':/config',hd+':/data',hr+':/work'])
        hm=[hc+':/config/host:ro',hd+':/data:rw',hr+':/run/rx-host:rw']
        if self.adapter:self.adapter_refusals(c,hm)
        d.command(os.environ.get('RX_LEGACY_HOST_INITIALIZER_IMAGE',s),'/opt/rx/bin/rx-hostd',['init','/config/host/startup.json'],hm,'h-init')
        host=d.start(s,'h','s','/opt/rx/bin/rx-hostd',['run','/config/host/startup.json'],hm)
        proxy_volume=d.volume('proxy-control');scratch=d.volume('proxy-data');work=d.volume('proxy-work')
        d.prepare_permissions(self.client_image,[proxy_volume+':/config',scratch+':/data',work+':/work'])
        proxy=d.start(self.client_image,'proxy','sdk-proxy','/opt/client-env/bin/python',['/proxy.py'],[str(ROOT/'clients/tests/loss_proxy.py')+':/proxy.py:ro',proxy_volume+':/control:rw'])
        template=json.loads((final/'executor-config/cell.template.json').read_text());transport=template['platform'];scope=template['expected_service']['scope']
        cfg={'target':'sdk-proxy:7443','server_name':transport['server_name'],'roots':transport['ca']['path'],'certificate':transport['certificate']['path'],'private_key':transport['key']['path']}
        publish_new(final/'executor-config/client.json',cfg)
        self.peer=Peer(d,c,self.language,final/'executor-config','sdk-peer')
        manifest=lambda name:hashlib.sha256(encoded(json.loads((ROOT/'spec'/name/'v1.0/protocol_manifest.json').read_text()))).hexdigest()
        self.base_hash=b64(manifest('contracts'));self.cell_hash=b64(manifest('cell_operations'));self.definition=b64(scope['definition'])
        self.hello={'peer_id':scope['principal'],'role':'ROLE_EXECUTOR','boot_id':uid(),'installation_id':scope['installation'],'store_generation':c['installation']['store_generation'],'release_digest':b64(scope['release']),'shared_clock_id':c['installation']['clock_id'],'supported_versions':[{'major':1,'schema_hash':self.base_hash}]}
        self.session=self.negotiate()
        return {'h':host,'e':self.peer.name,'h_data':hd,'e_data':scratch,'proxy':proxy,'proxy_control':proxy_volume}
    def adapter_refusals(self,c,mounts):
        d=self.docker;image=c['s_image']['Id'];private=c['materials'].temporary/'adapter-refusals';private.mkdir()
        original=json.loads((c['final']/'host-config/startup.json').read_text());rows=[]
        def refuse(label,value,expected,extra=()):
            path=private/(label+'.json');path.write_text(json.dumps(value));path.chmod(0o644)
            cmd=['docker','run','--rm','--read-only','--network','none','--cap-drop','ALL','--security-opt','no-new-privileges','--user','10001:10001']
            for mount in [*mounts,str(path)+':/negative.json:ro',*extra]:cmd+=['-v',mount]
            cmd+=['--entrypoint','/opt/rx/bin/rx-hostd',image,'inspect','/negative.json']
            r=subprocess.run(cmd,capture_output=True,text=True)
            rows.append({'label':label,'argv':cmd,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr})
            publish_new(d.evidence/(label+'.json'),rows[-1])
            assert r.returncode!=0 and expected in r.stdout+r.stderr,rows[-1]
        for label,key,value,expected in [('host-real-endpoint','endpoint','/dev/ttyUSB0','DXL_REAL_ENDPOINT_UNSUPPORTED'),('host-unknown-profile','profile','rx/other-driver','DXL_PROFILE_OR_SOURCE_UNSUPPORTED'),('host-wrong-source','driver_digest','11'*32,'DXL_PROFILE_OR_SOURCE_UNSUPPORTED')]:
            config=json.loads(json.dumps(original));config['backend'][key]=value;refuse(label,config,expected)
        bindings=json.loads((c['final']/'host-config/bindings.json').read_text());bindings[0]['allowed_intents'][0]['body']['program']['program']['sha256']='22'*32
        raw=encoded(bindings);bad=private/'bindings.json';bad.write_bytes(raw);bad.chmod(0o644)
        config=json.loads(json.dumps(original));config['bindings']={'path':'/negative/bindings.json','sha256':hashlib.sha256(raw).hexdigest()}
        refuse('host-wrong-ping-artifact',config,'DXL_PING_CONTRACT_REQUIRED',[str(bad)+':/negative/bindings.json:ro'])
        holder=d.holder(image,[]);binary=private/'helper';d.run('cp',holder+':/opt/rx/bin/rx-dynamixel-ping',str(binary));content=bytearray(binary.read_bytes());content[-1]^=1;binary.write_bytes(content);binary.chmod(0o755)
        refuse('host-helper-content-tamper',original,'release/content-mismatch',[str(binary)+':/opt/rx/bin/rx-dynamixel-ping:ro'])
        inventory=private/'inventory.json';d.run('cp',holder+':/opt/rx/manifests/runtime-files.json',str(inventory));value=json.loads(inventory.read_text());value['files']['bin/rx-dynamixel-ping']=hashlib.sha256(content).hexdigest();inventory.write_text(json.dumps(value));inventory.chmod(0o644)
        refuse('host-forged-helper-inventory',original,'release/content-mismatch',[str(binary)+':/opt/rx/bin/rx-dynamixel-ping:ro',str(inventory)+':/opt/rx/manifests/runtime-files.json:ro'])
        publish_new(d.evidence/'host-adapter-refusals.json',{'status':'HOST_ADAPTER_REFUSALS_PASS','rows':rows,'native_processes_started':0})
    def inspect_cell(self):
        return self.peer.call('rx.cell.v1.CellService/Inspect',{'context':self.context(),'cell_id':self.c['delivery']['cell']})
    def overview_run(self,run_id):
        overview=self.c['users']['operator'].get('/api/v1/overview')
        cell=next(v for v in overview['cells'] if v['cell']['value']['id']==self.c['delivery']['cell'])
        return next(v for v in cell['runs'] if v['value']['id']==run_id)
    def native_entries(self):
        script="import sqlite3,json; c=sqlite3.connect('file:/data/host/native-dynamixel/native.sqlite3?mode=ro',uri=True); print(json.dumps([json.loads(r[0])['value'] for r in c.execute(\"select document from entities where key like 'dynamixel-operation/%'\")]))"
        return json.loads(self.docker.run('exec',self.c['h'],'/usr/bin/python3','-c',script))
    def helper_calls(self):
        script="from pathlib import Path; p=Path('/data/host/native-dynamixel/helper-invocations.jsonl'); print(p.read_text() if p.exists() else '',end='')"
        raw=self.docker.run('exec',self.c['h'],'/usr/bin/python3','-c',script)
        return [json.loads(line) for line in raw.splitlines() if line.startswith('{')]
    def effects(self):
        if self.adapter:
            return [v for v in self.native_entries() if v['capture'] is not None]
        raw=self.docker.run('exec',self.c['h'],'/usr/bin/python3','-c',"from pathlib import Path; p=Path('/data/host/device/effects.jsonl'); print(p.read_text() if p.exists() else '',end='')")
        if raw and not raw.endswith('}'):
            return []
        return [json.loads(line) for line in raw.splitlines() if line]
    def proxy_mode(self,mode):
        script="import json;from pathlib import Path;p=Path('/control/mode.pending');p.write_text(json.dumps({'mode':"+repr(mode)+"}));p.replace('/control/mode.json')"
        self.docker.run('exec',self.c['proxy'],'/opt/client-env/bin/python','-c',script)
    def get_operation(self,operation):
        return self.peer.call('rx.contract.v1.OperationService/Get',{'context':self.context(),'operation_id':operation})
    def after_commissioning(self,c,active):
        self.c=c;operator=c['users']['operator'];cell_id=c['delivery']['cell']
        cell=self.inspect_cell();assert cell['commissioning']=='COMMISSIONING_COMMISSIONED',cell
        publish_new(self.docker.evidence/'sdk-commissioned-observation.json',{'language':self.language,'cell':cell,'scope':'real resident runtime/public commissioning; no robot connected'})
        configuration=operator.get('/api/v1/cell',id=cell_id);cfg=configuration['value']['configuration']
        run=operator.mutate('sdk-run-create','/api/v1/runs',{'cell':cell_id,'recipe_digest':cfg['recipe']['sha256'],'site_config_digest':cfg['site_config_digest'],'expected_cell':configuration['revision']})
        candidate=wait_for(lambda:operator.get('/api/v1/run/start-context',cell=cell_id,run=run['id'],purpose='PRODUCTION',budget_limit='1'),lambda v:v['can_request'])
        attempt=operator.mutate('sdk-run-start','/api/v1/runs/start',candidate['request'])
        started=wait_for(lambda:self.overview_run(run['id']),lambda r:r['value']['state']=='EXECUTING')
        current_cell=self.inspect_cell();budget=started['value']['budget'];mandate=started['value']['mandate']
        part=self.peer.call('rx.cell.v1.CellService/BeginPartAttempt',{'call':{'context':self.context(uid()),'cell_id':cell_id,'expected_cell_revision':current_cell['revision']},'run_id':run['id'],'mandate_id':mandate,'expected_budget_revision':budget['revision']})
        run_view=self.peer.call('rx.contract.v1.WorkflowService/GetRun',{'context':self.context(),'run_id':run['id']})
        resolved=json.loads((c['final']/'reference/resolved.json').read_text());node=resolved['root']['id']
        activation=self.peer.call('rx.contract.v1.WorkflowService/ResolveActivation',{'context':self.context(uid(),run_view['revision']),'run_id':run['id'],'node_id':node,'visit':'1'})
        run_view=self.peer.call('rx.contract.v1.WorkflowService/GetRun',{'context':self.context(),'run_id':run['id']});current_cell=self.inspect_cell()
        intent=json.loads(json.dumps(next(iter(resolved['bindings'].values()))['intent']))
        assert intent['kind']=='FINITE_ACTION' and set(intent['body'])=={'program'}
        # Fixture-only translation of published RX aliases to generated ProtoJSON.
        # No message definitions or runtime authority are implemented here.
        intent['kind']='KIND_FINITE_ACTION'
        for field in ['profile_digest','site_config_digest']:intent[field]=b64(intent[field])
        intent['calibration_digests']=[b64(v) for v in intent['calibration_digests']]
        for artifact in intent['body']['program'].values():artifact['sha256']=b64(artifact['sha256'])
        ctx=self.context(uid());request={'call':{'context':ctx,'cell_id':cell_id,'expected_cell_revision':current_cell['revision']},'request':{'context':ctx,'intent':intent,'run_id':run['id'],'activation_id':activation['activation_id'],'slot':'main'},'parent':{'mandate_id':mandate},'part_attempt_id':part['part_attempt_id'],'expected_run_revision':run_view['revision']}
        method='rx.cell.v1.CellService/SubmitOperation'
        # A valid request/session under a different certificate cannot inherit E authority.
        bad_dir=c['materials'].temporary/'wrong-identity';(bad_dir/'pki').mkdir(parents=True)
        browser=c['browser'];shutil.copy2(c['final']/browser['ca'],bad_dir/'pki/ca.pem');shutil.copy2(c['final']/browser['certificate'],bad_dir/'pki/client.pem');shutil.copy2(c['final']/browser['private_key'],bad_dir/'pki/client.key')
        publish_new(bad_dir/'client.json',{'target':'p:7443','server_name':'p','roots':'/config/executor/pki/ca.pem','certificate':'/config/executor/pki/client.pem','private_key':'/config/executor/pki/client.key'})
        bad=Peer(self.docker,c,self.language,bad_dir,'wrong-identity')
        refused=bad.call(method,request,ok=False);assert not refused['ok'] and refused['code']=='PERMISSION_DENIED',refused
        assert self.effects()==[];bad.close()
        forged=json.loads(json.dumps(request));forged['parent']['mandate_id']=uid()
        bad_context=self.context(uid());forged['call']['context']=bad_context;forged['request']['context']=bad_context
        forged_reply=self.peer.call(method,forged,ok=False);assert not forged_reply['ok'],forged_reply
        assert self.effects()==[]
        if self.mode=='unknown':
            if self.adapter:
                # Separate OS observer kills the helper during its fixed modeled latency.
                # No Host/helper fault switch or operation is replayed.
                script="import os,time,json,signal;from pathlib import Path;p=Path('/data/host/native-dynamixel/helper-invocations.jsonl');end=time.monotonic()+15\nwhile time.monotonic()<end:\n if p.exists():\n  rows=[json.loads(l) for l in p.read_text().splitlines() if l.startswith('{')]\n  if rows:\n   v=rows[-1];os.kill(v['pid'],signal.SIGKILL);print(json.dumps({'killed_helper':v}),flush=True);break\n time.sleep(.002)\nelse: raise RuntimeError('no helper observed')"
                observer=subprocess.Popen(['docker','exec',c['h'],'/usr/bin/python3','-c',script],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                receipt=self.peer.call(method,request)
                out,err=observer.communicate(timeout=20)
                publish_new(self.docker.evidence/'native-loss-observer.json',{'exit':observer.returncode,'stdout':out,'stderr':err})
                assert observer.returncode==0 and 'killed_helper' in out
            else:
                self.docker.run('exec',c['h'],'/bin/sh','-c',"touch /data/host/device/effects.jsonl && chmod 400 /data/host/device/effects.jsonl")
                receipt=self.peer.call(method,request)
        else:
            self.proxy_mode('drop-return')
            pending=self.peer.send({'method':method,'request':request,'timeout':10})
            observed=wait_for(self.effects,lambda values:len(values)==1,timeout=20)
            self.proxy_mode('cut');lost=self.peer.receive(pending,20)
            assert not lost['ok'] and lost['code'] in ('UNAVAILABLE','DEADLINE_EXCEEDED'),lost
            self.proxy_mode('pass');self.peer.action('reconnect');same=self.negotiate();assert same==self.session
            receipt=self.peer.call(method,request)
            assert observed[0]['operation']==receipt['operation_id']
        assert receipt['stage']=='RECEIPT_STAGE_ADMITTED',receipt
        again=self.peer.call(method,request);assert again==receipt
        operation=receipt['operation_id']
        if self.mode=='unknown':
            view=wait_for(lambda:self.get_operation(operation),lambda v:v.get('execution_knowledge')=='KNOWLEDGE_UNKNOWN' or v.get('outcome')=='OUTCOME_UNRESOLVED',timeout=30)
            assert view['outcome']!='OUTCOME_SUCCEEDED' and self.effects()==[]
        else:
            view=wait_for(lambda:self.get_operation(operation),lambda v:v.get('outcome')=='OUTCOME_SUCCEEDED',timeout=30)
            assert len(self.effects())==1
        self.peer.action('reconnect');same=self.negotiate();assert same==self.session
        after=self.get_operation(operation);assert after['operation_id']==operation and after['outcome']==view['outcome']
        other='cpp' if self.language=='python' else 'python'
        cross_peer=Peer(self.docker,c,other,c['final']/'executor-config','sdk-same-instance')
        cross_session=self.negotiate(peer=cross_peer);assert cross_session==self.session
        cross_receipt=cross_peer.call(method,request);assert cross_receipt==receipt
        cross_peer.close()
        self.peer.close()
        # A new client incarnation can query, but does not make a new work identity.
        self.peer=Peer(self.docker,c,self.language,c['final']/'executor-config','sdk-restarted')
        self.session=self.negotiate(boot=uid());restarted=self.get_operation(operation)
        assert restarted['operation_id']==operation and restarted['outcome']==view['outcome'],restarted
        self.peer.close()
        other='cpp' if self.language=='python' else 'python'
        self.peer=Peer(self.docker,c,other,c['final']/'executor-config','sdk-cross-language')
        retired=self.peer.call('rx.contract.v1.SessionService/Open',self.hello,ok=False)
        assert not retired['ok'] and retired['code']=='UNAUTHENTICATED',retired
        self.session=self.negotiate(boot=uid());cross=self.get_operation(operation)
        assert cross['operation_id']==operation
        if self.mode=='loss': assert cross['outcome']=='OUTCOME_SUCCEEDED'
        else: assert cross.get('execution_knowledge')=='KNOWLEDGE_UNKNOWN' or cross.get('outcome')=='OUTCOME_UNRESOLVED'
        self.peer.close()
        if self.adapter:
            calls=self.helper_calls();entries=self.native_entries();assert len(calls)==1 and len(entries)==1
            assert calls[0]['instance']==entries[0]['instance']
            publish_new(self.docker.evidence/'dynamixel-native-observation.json',{'calls':calls,'entries':entries,'cross_language_same_request_receipt':cross_receipt,'same_host':c['h'],'simulated_transport':True,'physical_qualification':'NOT_PERFORMED'})
        publish_new(self.docker.evidence/'result.json',{'status':'CLIENT_RUNTIME_PASS','language':self.language,'mode':self.mode,'platform_image':c['p_image']['Id'],'solutions_image':c['s_image']['Id'],'client_image':self.client_image,'runtime_clock':c['installation']['clock_id'],'receipt':receipt,'view':view,'after_client_restart':restarted,'cross_language':other,'cross_language_view':cross,'same_request_reply_preserved':True,'wrong_identity_refusal':refused,'forged_parent_refusal':forged_reply,'native_effects':self.effects(),'robot_connected':False,'simulated_adapter_connected':self.adapter,'physical_qualification':'NOT_PERFORMED','sdk_baseline_complete':False,'robotis_bundle_complete':False,'limitations':[('DYNAMIXEL read-only simulated Ping only' if self.adapter else 'FILE_SIMULATION, no ROBOTIS adapter; G5.2 required'),'Client process/transport exit is not operation cancellation','No clean whole-cell shutdown claim for deliberate unknown case']})

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--language',choices=['python','cpp'],required=True)
    parser.add_argument('--mode',choices=['loss','unknown'],required=True)
    parser.add_argument('--platform-image',required=True);parser.add_argument('--solutions-image',required=True);parser.add_argument('--client-image',required=True)
    parser.add_argument('--legacy-host-initializer-image',help='Cross-version FILE_SIMULATION persistence probe; only init uses this selected old image')
    parser.add_argument('--adapter-descriptor',type=Path,help='Offline selected S simulation adapter descriptor; not runtime authority')
    parser.add_argument('--release-evidence',type=Path,required=True);parser.add_argument('--evidence',type=Path,required=True)
    a=parser.parse_args();
    if a.legacy_host_initializer_image:
        if a.adapter_descriptor:parser.error('legacy initializer probe covers existing FILE_SIMULATION only')
        os.environ['RX_LEGACY_HOST_INITIALIZER_IMAGE']=a.legacy_host_initializer_image
    if a.adapter_descriptor:os.environ['RX_CELL_ADAPTER_DESCRIPTOR']=str(a.adapter_descriptor.resolve())
    e=a.evidence.absolute();e.mkdir(parents=True,exist_ok=False);d=Docker(e);scenario=Scenario(d,a.language,a.mode,d.image(a.client_image)['Id'])
    try:
        with tempfile.TemporaryDirectory(prefix='private-',dir=e) as directory:
            private=Path(directory);p=d.image(a.platform_image);s=d.image(a.solutions_image)
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
            bundle=private/'operator';holder=d.holder(s['Id'],[]);d.run('cp',holder+':/opt/rx/operator',str(bundle))
            materials=Materials(ROOT,private,e,d,s['Id']);materials.create_seed(s['Architecture']);package,compiled,compiler=materials.compile()
            identity=hashlib.sha256(Path(__file__).read_bytes()+(ROOT/'clients/tests/loss_proxy.py').read_bytes()).hexdigest()
            final=materials.finalize(package,compiled,compiler,port,bundle,identity)
            exercise(d,materials,final,bundle,p,s,port,identity,a.release_evidence.resolve(),'independent',start_services=scenario.start_services,after_commissioning=scenario.after_commissioning,expected_backend="VALIDATED_DRIVER" if scenario.adapter else "FILE_SIMULATION")
    finally:
        if scenario.peer and scenario.peer.process.poll() is None:
            scenario.peer.process.stdin.close()
        d.cleanup()

if __name__=='__main__':main()
