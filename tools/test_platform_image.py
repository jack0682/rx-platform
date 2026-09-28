#!/usr/bin/env python3
"""Run only the new uncommissioned platform image, using isolated disposable volumes/certificates."""
import argparse,hashlib,json,os,shutil,socket,ssl,subprocess,sys,tempfile,time,urllib.request,uuid
from pathlib import Path
parser=argparse.ArgumentParser();parser.add_argument('--image',default='rx-platform:runtime-draft');parser.add_argument('--evidence',required=True,type=Path);parser.add_argument('--package',type=Path);parser.add_argument('--package-policy',type=Path);parser.add_argument('--device-review-image');parser.add_argument('--device-review-source',type=Path);args=parser.parse_args()
assert bool(args.device_review_image)==bool(args.device_review_source) and (not args.device_review_image or args.package)
assert bool(args.package)==bool(args.package_policy), 'package and policy must be supplied together'
root=Path(__file__).resolve().parents[1]
def run(*argv,**kwargs):
    result=subprocess.run(argv,capture_output=True,text=True,**kwargs)
    if result.returncode:raise RuntimeError('test command failed: '+str(argv[0])+': '+result.stderr[-4000:]+' '+result.stdout[-1000:])
    return result.stdout.strip()
args.image=json.loads(run('docker','image','inspect',args.image))[0]['Id']
if args.device_review_image:args.device_review_image=json.loads(run('docker','image','inspect',args.device_review_image))[0]['Id']
with socket.socket() as s:
    s.bind(('127.0.0.1',0));port=s.getsockname()[1]
suffix=uuid.uuid4().hex[:12];config_volume='rx-platform-config-'+suffix;data_volume='rx-platform-data-'+suffix;setup='rx-platform-setup-'+suffix;service='rx-platform-smoke-'+suffix
with tempfile.TemporaryDirectory(prefix='rx-platform-image-') as temporary:
    fixture=Path(temporary)/'fixture'
    env=os.environ.copy();env.update(RX_PLATFORM_IMAGE_FIXTURE=str(fixture),RX_PLATFORM_IMAGE_PORT=str(port))
    if args.package and (args.package/'device-catalog.json').is_file():
        env['RX_PLATFORM_IMAGE_INSTALLATION']=json.loads((args.package/'device-catalog.json').read_text())['installation']
    run(str(root/'tools/cargo'),'test','-p','rx-platformd','--test','composition','export_container_fixture','--locked','--','--ignored','--exact',cwd=root,env=env)
    package_test=None
    if args.package:
        policy_bytes=args.package_policy.read_bytes();policy=json.loads(policy_bytes)
        assert not policy['dependencies'] and len(policy['assets'])<=32
        if policy['assets']:(fixture/'intake-assets').mkdir()
        for asset in policy['assets']:
            source=Path(asset['path']);reference=asset['reference']
            assert source.is_absolute() and source.is_file() and not source.is_symlink()
            raw=source.read_bytes();assert len(raw)<=1048576 and len(raw)==int(reference['size_bytes']) and hashlib.sha256(raw).hexdigest()==reference['sha256']
            (fixture/'intake-assets'/reference['sha256']).write_bytes(raw)
            asset['path']='/config/intake-assets/'+reference['sha256']
        if args.device_review_image:
            policy['schema']='rx.package-verification-policy.v2'
            policy['additional_package_abis']=['rx.package-abi.v1']
            policy['keys'][0]['kinds']=['DEVICE','PROCESS']
            policy['keys'][0]['permissions'].append({'kind':'OPERATION_SUBMIT','operation':'skill/1'})
        policy_bytes=json.dumps(policy,sort_keys=True,separators=(',',':')).encode()
        shutil.copytree(args.package,fixture/'intake'/'published');(fixture/'intake-policy.json').write_bytes(policy_bytes)
        startup=json.loads((fixture/'startup.json').read_bytes())
        catalog_path=fixture/Path(startup['catalog']['path']).name;catalog=json.loads(catalog_path.read_bytes())
        # Isolated fixture only: register the probe certificate as this test terminal.
        der=ssl.PEM_cert_to_DER_cert((fixture/'probe.pem').read_text())
        catalog['terminals'][0]['certificate_digest']=hashlib.sha256(der).hexdigest()
        if args.device_review_image:
            catalog['bootstrap']['roles'].append('VERIFIER')
            catalog['principals'].append({'id':'reviewer','client_namespace':'reviewer','roles':['VERIFIER','RELEASE_MANAGER'],'cells':['cell/a'],'active':True})
            credential_path=fixture/Path(startup['credentials']['path']).name
            credentials=json.loads(credential_path.read_bytes())
            credentials['accounts'].append({'principal':'reviewer','password_hash':credentials['accounts'][0]['password_hash']})
            raw=json.dumps(credentials,sort_keys=True,separators=(',',':')).encode();credential_path.write_bytes(raw)
            startup['credentials']['sha256']=hashlib.sha256(raw).hexdigest()

        catalog_path.write_text(json.dumps(catalog,sort_keys=True,separators=(',',':')))
        startup['catalog']['sha256']=hashlib.sha256(catalog_path.read_bytes()).hexdigest()
        startup['package_intake']={'import_root':'/config/intake','policy':{'path':'/config/intake-policy.json','sha256':hashlib.sha256(policy_bytes).hexdigest()}}
        if args.device_review_image:
            checker=json.loads(run('docker','run','--rm','--network','none','--entrypoint','/opt/rx/bin/rx-device-package',args.device_review_image,'validator-identity'))['validator_digest']
            authority={'schema':'rx.device-verification-authority.v1','keys':[{'id':'test/key','public_key':policy['keys'][0]['verifying_key'],'validators':[checker]}]}
            raw=json.dumps(authority,sort_keys=True,separators=(',',':')).encode();(fixture/'device-authority.json').write_bytes(raw)
            startup['package_intake']['device_review_authority']={'path':'/config/device-authority.json','sha256':hashlib.sha256(raw).hexdigest()}
            process_checker=json.loads(run('docker','run','--rm','--network','none','--entrypoint','/opt/rx/bin/rx-process-package',args.device_review_image,'validator-identity'))['validator_digest']
            authority={'schema':'rx.process-verification-authority.v1','keys':[{'id':'test/key','public_key':policy['keys'][0]['verifying_key'],'validators':[process_checker]}]}
            raw=json.dumps(authority,sort_keys=True,separators=(',',':')).encode();(fixture/'process-authority.json').write_bytes(raw)
            startup['package_intake']['review_authority']={'path':'/config/process-authority.json','sha256':hashlib.sha256(raw).hexdigest()}

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
            device_catalog=None
            if receipt.get('device_catalog'):
                device_catalog,_=api('/api/v1/package-intake/device-catalog?cell=cell%2Fa&id='+receipt['id'],cookie=cookie)
                original_catalog=json.loads((args.package/'device-catalog.json').read_text())
                assert device_catalog['catalog']==original_catalog and not device_catalog['activation_authorized']
            review_test=None
            if args.device_review_image:
                job,_=api('/api/v1/device-reviews',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'intake':receipt['id'],'cell':'cell/a','configuration_digest':context['configuration_digest'],'policy_generation':context['registration']['generation']}},cookie)
                review_request=Path(temporary)/'device-review-request.json';review_request.write_text(json.dumps(job['request']))
                run('docker','cp',str(review_request),setup+':/config/device-review-request.json')
                def checker_run(*argv):return run('docker','run','--rm','--user','0','--network','none','-v',config_volume+':/config','--entrypoint','/opt/rx/bin/rx-device-package',args.device_review_image,*argv)
                report=json.loads(checker_run('review','/config/intake/published','/config/intake-policy.json','/config/device-review-request.json','/config/intake/device-review'))
                assert report['software_checks_passed'] and not report['activation_authorized']
                checker_run('review-signing-request','/config/intake/device-review/verification.json','test/key','/config/device-report-signing.json')
                signing=Path(temporary)/'report-signing.json';signature=Path(temporary)/'report-signature.json'
                run('docker','cp',setup+':/config/device-report-signing.json',str(signing))
                signer_env=dict(os.environ,RX_PYTHON_SIGN_REQUEST=str(signing),RX_PYTHON_SIGN_OUTPUT=str(signature),CARGO_TARGET_DIR=str(args.device_review_source.resolve().parent/'.build/python-host-bridge'))
                run(str(args.device_review_source.resolve()/'tools/cargo'),'test','-p','rx-device-package','--test','python','sign_python_fixture_message','--locked','--','--ignored','--exact',cwd=args.device_review_source,env=signer_env)
                run('docker','cp',str(signature),setup+':/config/intake/device-review/verification.sig.json')
                run('docker','run','--rm','--user','0','--network','none','-v',config_volume+':/config','--entrypoint','/bin/sh',args.image,'-c','chown -R 10001:10001 /config/intake/device-review; chmod -R u+rwX,go-rwx /config/intake/device-review')
                report_request={'request_key':str(uuid.uuid4()),'command':{'review':job['request']['id'],'cell':'cell/a','expected':None,'directory':'device-review','report_digest':report['report_digest']}}
                version,_=api('/api/v1/device-review/reports',report_request,cookie)
                repeat_version,_=api('/api/v1/device-review/reports',report_request,cookie)
                assert version==repeat_version and version['ready_for_software_approval']
                detail,_=api('/api/v1/device-review?cell=cell%2Fa&id='+job['request']['id'],cookie=cookie)
                assert detail['decision'] is None and not detail['activation_authorized']
                decision_command={'review':job['request']['id'],'cell':'cell/a','report_revision':version['revision'],'review_digest':version['review_digest'],'expected':None,'choice':'APPROVE','note':'Separate test account reviewed exact software report; no physical qualification.'}
                try:api('/api/v1/device-review/decisions',{'request_key':str(uuid.uuid4()),'command':decision_command},cookie)
                except urllib.error.HTTPError as denied:assert denied.code==403
                else:raise AssertionError('submitter self-approval accepted')
                _,reviewer_headers=api('/api/v1/session',{'principal':'reviewer','password':'composition-test-password'})
                reviewer_cookie=reviewer_headers['set-cookie'].split(';')[0]
                decision,_=api('/api/v1/device-review/decisions',{'request_key':str(uuid.uuid4()),'command':decision_command},reviewer_cookie)
                cell,_=api('/api/v1/cell?id=cell%2Fa',cookie=cookie)
                required=device_catalog['catalog']['condition_ids']
                assert required==['sim/ready'], 'this Python fixture supports one explicitly declared readiness condition'
                plan_input={'id':str(uuid.uuid4()),'cell':'cell/a','review':{'id':job['request']['id'],'revision':version['revision'],'review_digest':version['review_digest'],'decision_revision':decision['revision']},
                    'bindings':{'skill/python':{'action':'skill/run','host':cell['value']['configuration']['hosts'][0],
                        'conditions':{name:cell['value']['configuration']['start_conditions'][0] for name in required},'completion_postconditions':[], 'handover_max_age_ns':'1000000000'}},
                    'reason':'Bind the reviewed Python SDK declaration; no deployment or qualification inferred.'}
                plan,_=api('/api/v1/device-binding-plans',{'request_key':str(uuid.uuid4()),'command':plan_input},cookie)
                assert not plan['definition']['issues'],plan['definition']['issues']
                reviewed_plan,_=api('/api/v1/device-binding-plan/impact-review',{'request_key':str(uuid.uuid4()),'command':{'plan':plan['id'],'cell':'cell/a','expected':plan['revision'],'plan_digest':plan['plan_digest'],'note':'Checked isolated simulation Host/resource scope.'}},reviewer_cookie)
                assert reviewed_plan['state']=='IMPACT_REVIEWED'
                selected,_=api('/api/v1/process-draft/binding-options',{'cell':'cell/a','device_plans':[{'id':reviewed_plan['id'],'revision':reviewed_plan['revision'],'plan_digest':reviewed_plan['plan_digest']}]},cookie)
                assert any(c['step']=='skill/python' and c['device_plan'] is not None for c in selected['candidates'])
                approved_detail,_=api('/api/v1/device-review?cell=cell%2Fa&id='+job['request']['id'],cookie=cookie)
                assert approved_detail['approval_matches_current_review'] and not approved_detail['activation_authorized']
                cli_holder='rx-python-cli-'+suffix
                try:
                    run('docker','create','--name',cli_holder,'--entrypoint','/bin/true',args.device_review_image)
                    client=Path(temporary)/'installed-client';run('docker','cp',cli_holder+':/opt/rx/client',str(client))
                finally:subprocess.run(['docker','rm','-f',cli_holder],capture_output=True)
                for name in ['rx','runtime_client.py']:
                    assert (client/name).read_bytes()==(args.device_review_source/'deployment/local-skills'/name).read_bytes()
                (fixture/'probe.key').chmod(0o600)
                password=Path(temporary)/'author-password';password.write_text('composition-test-password');password.chmod(0o600)
                connection=Path(temporary)/'author-connection.json';connection.write_text(json.dumps({'schema':'rx.runtime-skill-connection.v1','origin':origin,'ca':str(fixture/'ca.pem'),'certificate':str(fixture/'probe.pem'),'private_key':str(fixture/'probe.key'),'principal':'admin','password_file':str(password)}));connection.chmod(0o600)
                plan_file=Path(temporary)/'reviewed-plan.json';plan_file.write_text(json.dumps(reviewed_plan))
                compose_id=str(uuid.uuid4());cli=[sys.executable,str(client/'rx'),'runtime','--connection',str(connection),'--state-dir',str(Path(temporary)/'author-journal')]
                composition=json.loads(run(*cli,'compose','python-composition','--cell','cell/a','--step','skill/python','--device-plan',str(plan_file),'--request-id',compose_id))
                recovered_composition=json.loads(run(*cli,'compose-recover',compose_id))
                assert composition==recovered_composition and composition['status']=='DRAFT_READY_FOR_COMPILER'
                provenance=composition['compile_input']['device_sources']['skill/1']
                assert provenance['plan']['id']==reviewed_plan['id'] and provenance['binding']=='skill/python'
                def process_cli(*argv):return run('docker','run','--rm','--user','0','--network','none','-v',config_volume+':/config','--entrypoint','/opt/rx/bin/rx-process-package',args.device_review_image,*argv)
                def put_json(filename,value):
                    local=Path(temporary)/filename;local.write_text(json.dumps(value));run('docker','cp',str(local),setup+':/config/'+filename)
                def sign_public_request(remote,name):
                    source=Path(temporary)/(name+'.json');output=Path(temporary)/(name+'.sig.json')
                    run('docker','cp',setup+':'+remote,str(source))
                    env=dict(signer_env,RX_PYTHON_SIGN_REQUEST=str(source),RX_PYTHON_SIGN_OUTPUT=str(output))
                    run(str(args.device_review_source.resolve()/'tools/cargo'),'test','-p','rx-device-package','--test','python','sign_python_fixture_message','--locked','--','--ignored','--exact',cwd=args.device_review_source,env=env)
                    return output
                manifest=receipt['manifest'];contracts=dict(manifest['contracts'],package_abi='rx.package-abi.v1')
                recipe={'schema':'rx.process-package-recipe.v1','package':'test/python-process','version':'1.0.0','publisher':'test','contracts':contracts,'targets':manifest['targets'],'dependencies':[],'assets':manifest['assets']}
                put_json('python-compile-input.json',composition['compile_input']);put_json('python-process-recipe.json',recipe)
                process_candidate=json.loads(process_cli('assemble','/config/python-compile-input.json','/config/python-process-recipe.json','/config/python-process-candidate'))
                process_cli('request','/config/python-process-candidate','test/key','/config/python-process-signing.json')
                signed=sign_public_request('/config/python-process-signing.json','python-process')
                run('docker','cp',str(signed),setup+':/config/python-process.sig.json')
                process_cli('seal','/config/python-process-candidate','/config/python-process.sig.json','/config/intake-policy.json','/config/intake/python-process')
                run('docker','run','--rm','--user','0','--network','none','-v',config_volume+':/config','--entrypoint','/bin/sh',args.image,'-c','chown -R 10001:10001 /config/intake/python-process; chmod -R u+rwX,go-rwx /config/intake/python-process')
                signature_hash=hashlib.sha256(signed.read_bytes()).hexdigest()
                process_intake,_=api('/api/v1/package-intakes',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'cell':'cell/a','title':'Reviewed Python process','relative_path':'python-process','object':{'manifest':process_candidate['manifest_digest'],'signature':signature_hash},'configuration_digest':context['configuration_digest'],'policy_generation':context['registration']['generation']}},cookie)
                process_job,_=api('/api/v1/process-reviews',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'intake':process_intake['id'],'cell':'cell/a','configuration_digest':context['configuration_digest'],'policy_generation':context['registration']['generation'],'binding_selections':{'skill/1':'skill/python'},'device_plans':[provenance['plan']]}},cookie)
                put_json('python-process-review-request.json',process_job['request'])
                process_report=json.loads(process_cli('review','/config/intake/python-process','/config/intake-policy.json','/config/python-process-review-request.json','/config/intake/python-process-review'))
                assert process_report['compiler_checks_passed']
                process_cli('review-signing-request','/config/intake/python-process-review/verification.json','test/key','/config/python-process-report-signing.json')
                signed_report=sign_public_request('/config/python-process-report-signing.json','python-process-report')
                run('docker','cp',str(signed_report),setup+':/config/intake/python-process-review/verification.sig.json')
                run('docker','run','--rm','--user','0','--network','none','-v',config_volume+':/config','--entrypoint','/bin/sh',args.image,'-c','chown -R 10001:10001 /config/intake/python-process /config/intake/python-process-review; chmod -R u+rwX,go-rwx /config/intake/python-process /config/intake/python-process-review')
                process_version,_=api('/api/v1/process-review/reports',{'request_key':str(uuid.uuid4()),'command':{'review':process_job['request']['id'],'cell':'cell/a','expected':None,'directory':'python-process-review','report_digest':process_report['report_digest']}},cookie)
                process_decision,_=api('/api/v1/process-review/decisions',{'request_key':str(uuid.uuid4()),'command':{'review':process_job['request']['id'],'cell':'cell/a','report_revision':process_version['revision'],'review_digest':process_version['review_digest'],'expected':None,'choice':'APPROVE','note':'Separate fixture reviewer checked signed Python process.'}},reviewer_cookie)
                change,_=api('/api/v1/process-changes',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'cell':'cell/a','mode':'REPLACE','review':{'id':process_job['request']['id'],'revision':process_version['revision'],'review_digest':process_version['review_digest'],'decision_revision':process_decision['revision']},'reason':'Prepare the reviewed Python process deployment.'}},cookie)
                assert change['host_binding_plan'] is not None
                def transition(value):return {'change':value['id'],'cell':'cell/a','expected':value['revision'],'plan_digest':value['plan_digest']}
                change,_=api('/api/v1/process-change/impact-review',{'request_key':str(uuid.uuid4()),'command':{'target':transition(change),'note':'Review isolated simulation binding replacement.'}},reviewer_cookie)
                change,_=api('/api/v1/process-change/stage',{'request_key':str(uuid.uuid4()),'command':transition(change)},reviewer_cookie)
                binding_intent_request={'request_key':str(uuid.uuid4()),'command':transition(change)}
                try:api('/api/v1/process-change/host-binding-intents',binding_intent_request,cookie)
                except urllib.error.HTTPError as denied:assert denied.code==403
                else:raise AssertionError('binding intent issuance accepted without ReleaseManager role')
                binding_intents,_=api('/api/v1/process-change/host-binding-intents',binding_intent_request,reviewer_cookie)
                same_intents,_=api('/api/v1/process-change/host-binding-intents',binding_intent_request,reviewer_cookie)
                assert binding_intents==same_intents and len(binding_intents)==len(change['host_binding_plan']['hosts'])
                conflicting={'request_key':binding_intent_request['request_key'],'command':dict(binding_intent_request['command'],plan_digest='00'*32)}
                try:api('/api/v1/process-change/host-binding-intents',conflicting,reviewer_cookie)
                except urllib.error.HTTPError as denied:assert denied.code==409
                else:raise AssertionError('binding request key accepted changed content')

                for intent in binding_intents:
                    assert intent['phase']=='AWAITING_BASELINE' and intent['baseline'] is None and not intent['activation_authorized']
                    assert intent['intent']['change']==change['id'] and intent['intent']['before_configuration']==change['before']['sha256'] and intent['intent']['after_configuration']==change['after']['sha256']
                try:api('/api/v1/process-change/prepare',{'request_key':str(uuid.uuid4()),'command':{'target':transition(change),'refresh':False}},reviewer_cookie)
                except urllib.error.HTTPError as blocked:
                    assert blocked.code in (409,422);deployment_denial={'status':blocked.code,'body':json.loads(blocked.read())}
                else:raise AssertionError('Host binding replacement guard unexpectedly passed')
                change_detail,_=api('/api/v1/process-change?cell=cell%2Fa&id='+change['id'],cookie=cookie)
                assert any(v['kind']=='HOST_BINDING_CHANGE_REQUIRED' for v in change_detail['blockers'])
                deployment={'process_intake':process_intake,'process_report':process_report,'process_decision':process_decision,'change':change_detail,'guard':deployment_denial,'status':'HOST_BINDING_CHANGE_REQUIRED','binding_intents':binding_intents,'same_intents_on_retry':True}


                final_cell,_=api('/api/v1/cell?id=cell%2Fa',cookie=cookie)
                assert final_cell['value']['configuration']==cell['value']['configuration']
                final_overview,_=api('/api/v1/overview',cookie=cookie)
                assert all(not c['runs'] and c['cell']['value']['qualification'] is None for c in final_overview['cells'])

                review_test={'request':job['request'],'report':report,'version':version,'pre_approval_detail':detail,'detail':approved_detail,'repeat_returns_same_report_version':True,'self_approval_denied':True,'decision':decision,'binding_plan':reviewed_plan,'binding_options':selected,'composition':composition,'compose_recovery_preserved':True,'deployment':deployment}
            package_test={'status':'PASS','receipt':receipt,'view':page['packages'][0],'repeat_returns_identical_receipt':True,'no_run_or_qualification':True,'policy_sha256':hashlib.sha256(policy_bytes).hexdigest(),'device_catalog':device_catalog,'device_review':review_test}
        run('docker','stop','--time','15',service)
        stopped=json.loads(run('docker','inspect',service))[0];assert stopped['State']['ExitCode']==0,run('docker','logs',service)
        run('docker','cp',service+':/data/runtime/platform-status.json',str(Path(temporary)/'stopped.json'))
        after=json.loads((Path(temporary)/'stopped.json').read_text());assert after['phase']=='PROCESS_STOPPED';assert after['stop']['lifecycle']['phase']=='STOP_COMMITTED';assert not after['physical_shutdown_assessed']
        image=json.loads(run('docker','image','inspect',args.image))[0]
        result={'schema':'rx.platform-image-smoke.v1','status':'PASS','image_id':image['Id'],'os':image['Os'],'architecture':image['Architecture'],'user':inspected['Config']['User'],'read_only_root':True,'cap_drop':['ALL'],'https_health':health,'startup':before,'stop':after,'package_intake':package_test,'device_review_image':args.device_review_image,'limitations':['uncommissioned draft authority','no Host/controller launch','no physical shutdown qualification','config/data volumes and test certificates were disposable']}
        args.evidence.parent.mkdir(parents=True,exist_ok=True);args.evidence.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','image':image['Id']}))
    finally:
        for name in [service,setup]:subprocess.run(['docker','rm','-f',name],capture_output=True)
        for name in [config_volume,data_volume]:subprocess.run(['docker','volume','rm',name],capture_output=True)
