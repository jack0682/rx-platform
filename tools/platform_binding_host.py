"""Isolated FILE_SIMULATION Host for the live Platform binding-reader acceptance."""
import hashlib
import json
from pathlib import Path
import ssl
import subprocess
import time
import uuid

def encoded(value):return json.dumps(value,sort_keys=True,separators=(',',':')).encode()
class BindingHost:
    def __init__(self,run,fixture,image,token):
        self.run,self.image=run,image
        self.network=token+'-network';self.setup=token+'-host-setup';self.service=token+'-host'
        self.volumes=[token+'-host-config',token+'-host-data'];self.fixture=fixture/'binding-host';self.fixture.mkdir()
        startup=json.loads((fixture/'startup.json').read_text());catalog=json.loads((fixture/Path(startup['catalog']['path']).name).read_text())
        cell=next(c for c in catalog['cells'] if c['id']=='cell/a');host=cell['hosts'][0]
        bindings=[{'host':host,'platform':startup['installation_id'],'cell':cell['id'],'definition':cell['definition'],'envelope':cell['envelope'],
            'qualification':str(uuid.uuid4()),'qualification_revision':'1','allowed_intents':[s['intent'] for s in cell['steps'] if s['host']==host],
            'scope_ids':cell['scopes'],'condition_ids':sorted({v for s in cell['steps'] for v in s['condition_ids']}),'environment':'SIMULATION','purposes':['PRODUCTION']}]
        def put(name,raw):
            (self.fixture/name).write_bytes(raw);return {'path':'/config/'+name,'sha256':hashlib.sha256(raw).hexdigest()}
        binding_pin=put('bindings.json',encoded(bindings))
        cert=Path(startup['https']['tls']['certificate']['path']).name;key=Path(startup['https']['tls']['key']['path']).name;ca=Path(startup['https']['tls']['ca']['path']).name
        server=put('server.pem',(fixture/cert).read_bytes());private=put('server.key',(fixture/key).read_bytes());authority=put('ca.pem',(fixture/ca).read_bytes())
        link=startup['host_links'][0]
        client=(fixture/Path(link['tls']['certificate']['path']).name).read_text()
        fingerprint=hashlib.sha256(ssl.PEM_cert_to_DER_cert(client)).hexdigest()
        config={'schema':'rx.host-startup.v1','installation':startup['installation_id'],'release_digest':startup['release_digest'],'host':host,'bind':'0.0.0.0:7444',
            'data_directory':'/data/host','runtime_directory':'/data/runtime','bindings':binding_pin,'backend':{'kind':'FILE_SIMULATION'},
            'tls':{'certificate':server,'key':private,'ca':authority},'allowed_platform_certificates':{fingerprint:startup['installation_id']},'publisher':None,'publication_drain_ms':'0'}
        put('startup.json',encoded(config))
        link['uri']='https://binding-host:7444';link['server_name']='localhost'
        link['server_fingerprint']=hashlib.sha256(ssl.PEM_cert_to_DER_cert((fixture/cert).read_text())).hexdigest()
        catalog['principals'].append({'id':host,'client_namespace':host,'roles':['HOST'],'cells':[cell['id']],'active':True})
        catalog_path=fixture/Path(startup['catalog']['path']).name
        raw=encoded(catalog);catalog_path.write_bytes(raw);startup['catalog']['sha256']=hashlib.sha256(raw).hexdigest()
        startup['grpc']['allowed_certificates'][fingerprint]=host
        publisher_cert=put('publisher.pem',(fixture/Path(link['tls']['certificate']['path']).name).read_bytes())
        publisher_key=put('publisher.key',(fixture/Path(link['tls']['key']['path']).name).read_bytes())
        self.publisher={'uri':'https://platform:7443','server_name':'localhost','tls':{'certificate':publisher_cert,'key':publisher_key,'ca':authority}}
        self.config=config
        (fixture/'startup.json').write_bytes(encoded(startup))
    def create_network(self):self.run('docker','network','create',self.network)
    def start(self,store_generation):
        self.config['publisher']=dict(self.publisher,store_generation=store_generation)
        (self.fixture/'startup.json').write_bytes(encoded(self.config))
        r=self.run
        for v in self.volumes:r('docker','volume','create',v)
        r('docker','create','--name',self.setup,'--user','0','--network','none','-v',self.volumes[0]+':/config','-v',self.volumes[1]+':/data',
            '--entrypoint','/bin/sh',self.image,'-c','mkdir -p /data/runtime; chmod -R u+rwX,go-rwx /config; /opt/rx/bin/rx-hostd init /config/startup.json && chown -R 10001:10001 /config /data')
        r('docker','cp',str(self.fixture)+'/.',self.setup+':/config');r('docker','start','--attach',self.setup)
        assert json.loads(r('docker','inspect',self.setup))[0]['State']['ExitCode']==0
        r('docker','run','-d','--name',self.service,'--network',self.network,'--network-alias','binding-host','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw',
            '-v',self.volumes[0]+':/config:ro','-v',self.volumes[1]+':/data','--entrypoint','/opt/rx/bin/rx-hostd',self.image,'run','/config/startup.json')
        until=time.monotonic()+20
        while True:
            state=json.loads(r('docker','inspect',self.service))[0];assert state['State']['Running'],r('docker','logs',self.service)
            reply=subprocess.run(['docker','exec',self.service,'cat','/data/runtime/host-status.json'],capture_output=True,text=True)
            if reply.returncode==0:
                ready=json.loads(reply.stdout)
                if ready['phase']=='SOFTWARE_READY_UNARMED':return ready
            assert time.monotonic()<until;time.sleep(.1)
    def close(self):
        subprocess.run(['docker','stop','--time','10',self.service],capture_output=True)
        for n in [self.service,self.setup]:subprocess.run(['docker','rm','-f',n],capture_output=True)
        for n in self.volumes:subprocess.run(['docker','volume','rm',n],capture_output=True)
        subprocess.run(['docker','network','rm',self.network],capture_output=True)
