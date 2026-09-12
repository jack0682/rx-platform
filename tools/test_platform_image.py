#!/usr/bin/env python3
"""Run only the new uncommissioned platform image, using isolated disposable volumes/certificates."""
import argparse,hashlib,json,os,shutil,socket,ssl,subprocess,tempfile,time,urllib.request,uuid
from pathlib import Path
parser=argparse.ArgumentParser();parser.add_argument('--image',default='rx-platform:runtime-draft');parser.add_argument('--evidence',required=True,type=Path);parser.add_argument('--package',type=Path);parser.add_argument('--package-policy',type=Path);args=parser.parse_args()
assert bool(args.package)==bool(args.package_policy), 'package and policy must be supplied together'
root=Path(__file__).resolve().parents[1]
def run(*argv,**kwargs):
    return subprocess.run(argv,check=True,capture_output=True,text=True,**kwargs).stdout.strip()
with socket.socket() as s:
    s.bind(('127.0.0.1',0));port=s.getsockname()[1]
suffix=uuid.uuid4().hex[:12];config_volume='rx-platform-config-'+suffix;data_volume='rx-platform-data-'+suffix;setup='rx-platform-setup-'+suffix;service='rx-platform-smoke-'+suffix
with tempfile.TemporaryDirectory(prefix='rx-platform-image-') as temporary:
    fixture=Path(temporary)/'fixture'
    env=os.environ.copy();env.update(RX_PLATFORM_IMAGE_FIXTURE=str(fixture),RX_PLATFORM_IMAGE_PORT=str(port))
    run(str(root/'tools/cargo'),'test','-p','rx-platformd','--test','composition','export_container_fixture','--locked','--','--ignored','--exact',cwd=root,env=env)
    package_test=None
    if args.package:
        policy_bytes=args.package_policy.read_bytes();policy=json.loads(policy_bytes)
        assert not policy['dependencies'] and not policy['assets'], 'Use a self-contained canonical test package'
        shutil.copytree(args.package,fixture/'intake'/'published');(fixture/'intake-policy.json').write_bytes(policy_bytes)
        startup=json.loads((fixture/'startup.json').read_bytes())
        catalog_path=fixture/Path(startup['catalog']['path']).name;catalog=json.loads(catalog_path.read_bytes())
        # Isolated fixture only: register the probe certificate as this test terminal.
        der=ssl.PEM_cert_to_DER_cert((fixture/'probe.pem').read_text())
        catalog['terminals'][0]['certificate_digest']=hashlib.sha256(der).hexdigest()
        catalog_path.write_text(json.dumps(catalog,sort_keys=True,separators=(',',':')))
        startup['catalog']['sha256']=hashlib.sha256(catalog_path.read_bytes()).hexdigest()
        startup['package_intake']={'import_root':'/config/intake','policy':{'path':'/config/intake-policy.json','sha256':hashlib.sha256(policy_bytes).hexdigest()}}
        (fixture/'startup.json').write_text(json.dumps(startup,sort_keys=True,separators=(',',':')))
    try:
        for name in [config_volume,data_volume]:run('docker','volume','create',name)
        run('docker','create','--name',setup,'--user','0','--network','none','-v',config_volume+':/config','-v',data_volume+':/data','--entrypoint','/bin/sh',args.image,'-c','chmod 700 /config /data; chmod -R u+rwX,go-rwx /config; mkdir -p /data/runtime; /usr/local/bin/rx-platformd init /config/startup.json && chown -R 10001:10001 /data /config')
        run('docker','cp',str(fixture)+'/.',setup+':/config');run('docker','start','--attach',setup)
        setup_state=json.loads(run('docker','inspect',setup))[0]['State'];assert setup_state['ExitCode']==0,setup_state
        run('docker','run','-d','--name',service,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',config_volume+':/config:ro','-v',data_volume+':/data:rw','-p',f'127.0.0.1:{port}:8443',args.image,'run','/config/startup.json')
        context=ssl.create_default_context(cafile=str(fixture/'ca.pem'));context.load_cert_chain(str(fixture/'probe.pem'),str(fixture/'probe.key'))
        deadline=time.monotonic()+20;health=None;last_error=None
        opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPSHandler(context=context))
        while time.monotonic()<deadline:
            state=json.loads(run('docker','inspect',service))[0]
            assert state['State']['Running'],run('docker','logs',service)
            try:
                with opener.open(f'https://127.0.0.1:{port}/api/v1/health',timeout=2) as response:health=json.load(response)
                if health.get('admission_open'):break
            except (OSError,urllib.error.URLError) as error:last_error=str(error)
            time.sleep(.1)
        if health is None:
            print(json.dumps({'last_https_error':last_error,'container_logs':run('docker','logs',service),'process_status':run('docker','exec',service,'cat','/data/runtime/platform-status.json')}))
        assert health and health['writer_available'] and health['admission_open'],health
        inspected=json.loads(run('docker','inspect',service))[0]
        assert inspected['Config']['User']=='10001:10001';assert inspected['HostConfig']['ReadonlyRootfs'];assert inspected['HostConfig']['CapDrop']==['ALL']
        before=json.loads(run('docker','exec',service,'cat','/data/runtime/platform-status.json'))
        assert before['phase']=='SOFTWARE_READY_UNCOMMISSIONED';assert before['qualification_authority']=='NOT_CONNECTED'
        if args.package:
            origin=f'https://127.0.0.1:{port}'
            def api(path,body=None,cookie=None):
                headers={}
                if body is not None:headers.update({'origin':origin,'x-rx-client':'browser-v1','content-type':'application/json'})
                if cookie:headers['cookie']=cookie
                req=urllib.request.Request(origin+path,data=None if body is None else json.dumps(body).encode(),headers=headers)
                with opener.open(req,timeout=10) as response:return json.load(response),response.headers
            _,headers=api('/api/v1/session',{'principal':'admin','password':'composition-test-password'})
            cookie=headers['set-cookie'].split(';')[0]
            context,_=api('/api/v1/package-intake-context?cell=cell%2Fa',cookie=cookie)
            # This smoke uses the canonical files published by the S package tool.
            obj={'manifest':hashlib.sha256((args.package/'manifest.json').read_bytes()).hexdigest(),'signature':hashlib.sha256((args.package/'manifest.sig.json').read_bytes()).hexdigest()}
            command={'id':str(uuid.uuid4()),'cell':'cell/a','title':'Image acceptance package','relative_path':'published','object':obj,'configuration_digest':context['configuration_digest'],'policy_generation':context['registration']['generation']}
            request={'request_key':str(uuid.uuid4()),'command':command}
            receipt,_=api('/api/v1/package-intakes',request,cookie)
            repeated,_=api('/api/v1/package-intakes',request,cookie)
            assert receipt==repeated and receipt['state']=='AWAITING_REVIEW' and receipt['terminal']=='panel/a'
            page,_=api('/api/v1/package-intakes?cell=cell%2Fa',cookie=cookie)
            assert len(page['packages'])==1 and not page['packages'][0]['activation_authorized'] and page['packages'][0]['content_reverification_required']
            overview,_=api('/api/v1/overview',cookie=cookie)
            assert all(not c['runs'] and c['cell']['value']['qualification'] is None for c in overview['cells'])
            package_test={'status':'PASS','receipt':receipt,'view':page['packages'][0],'repeat_returns_identical_receipt':True,'no_run_or_qualification':True,'policy_sha256':hashlib.sha256(args.package_policy.read_bytes()).hexdigest()}
        run('docker','stop','--time','15',service)
        stopped=json.loads(run('docker','inspect',service))[0];assert stopped['State']['ExitCode']==0,run('docker','logs',service)
        run('docker','cp',service+':/data/runtime/platform-status.json',str(Path(temporary)/'stopped.json'))
        after=json.loads((Path(temporary)/'stopped.json').read_text());assert after['phase']=='PROCESS_STOPPED';assert after['stop']['lifecycle']['phase']=='STOP_COMMITTED';assert not after['physical_shutdown_assessed']
        image=json.loads(run('docker','image','inspect',args.image))[0]
        result={'schema':'rx.platform-image-smoke.v1','status':'PASS','image_id':image['Id'],'os':image['Os'],'architecture':image['Architecture'],'user':inspected['Config']['User'],'read_only_root':True,'cap_drop':['ALL'],'https_health':health,'startup':before,'stop':after,'package_intake':package_test,'limitations':['uncommissioned draft authority','no Host/controller launch','no physical shutdown qualification','config/data volumes and test certificates were disposable']}
        args.evidence.parent.mkdir(parents=True,exist_ok=True);args.evidence.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','image':image['Id']}))
    finally:
        for name in [service,setup]:subprocess.run(['docker','rm','-f',name],capture_output=True)
        for name in [config_volume,data_volume]:subprocess.run(['docker','volume','rm',name],capture_output=True)
