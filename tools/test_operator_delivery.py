#!/usr/bin/env python3
"""Actual two-image UI delivery and direct terminal mTLS. Disposable SIMULATION fixture only."""
import argparse
import hashlib
import json
import os
import socket
import ssl
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

parser=argparse.ArgumentParser()
parser.add_argument('--platform-image',default='rx-platform:runtime-draft')
parser.add_argument('--solutions-image',default='rx-solutions:runtime-draft')
parser.add_argument('--evidence-dir',required=True,type=Path)
args=parser.parse_args()
args.evidence_dir.mkdir(parents=True,exist_ok=False)
out=args.evidence_dir.resolve();root=Path(__file__).resolve().parents[1]
def run(*cmd,**kwargs):
    result=subprocess.run(cmd,capture_output=True,text=True,check=True,**kwargs)
    return result.stdout.strip()
def state(container):return json.loads(run('docker','inspect',container))[0]
suffix=uuid.uuid4().hex[:12]
names={key:'rx-operator-'+key+'-'+suffix for key in ['source','setup','service','config','data','assets']}
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
with tempfile.TemporaryDirectory(prefix='rx-operator-delivery-') as temporary:
    temp=Path(temporary).resolve();bundle=temp/'operator';fixture=temp/'fixture'
    p_image=json.loads(run('docker','image','inspect',args.platform_image))[0]
    s_image=json.loads(run('docker','image','inspect',args.solutions_image))[0]
    try:
        run('docker','create','--name',names['source'],'--network','none','--entrypoint','/bin/true',s_image['Id'])
        run('docker','cp',names['source']+':/opt/rx/operator',str(bundle))
        manifest=json.loads((bundle/'operator-bundle.json').read_text())
        pin=hashlib.sha256((bundle/'operator-bundle.json').read_bytes()).hexdigest()
        assert manifest['schema']=='rx.operator-ui-bundle.v1'
        assert all(any(f['path'].endswith(ext) for f in manifest['files']) for ext in ['.js','.css','.woff2'])
        env=dict(os.environ,CARGO_INCREMENTAL='0',RX_PLATFORM_IMAGE_FIXTURE=str(fixture),RX_PLATFORM_IMAGE_PORT=str(port),RX_PLATFORM_OPERATOR_BUNDLE=str(bundle))
        run(str(root/'tools/cargo'),'test','-p','rx-platformd','--test','composition','export_container_fixture','--locked','--offline','--','--ignored','--exact',cwd=root,env=env)
        browser_data=json.loads((fixture/'browser-fixture.json').read_text())
        assert browser_data['simulation_only']
        origin=browser_data['origin'];assert origin==f'https://127.0.0.1:{port}'
        for key in ['config','data','assets']:run('docker','volume','create',names[key])
        run('docker','create','--name',names['setup'],'--user','0','--network','none',
            '-v',names['config']+':/config','-v',names['data']+':/data','-v',names['assets']+':/operator',
            '--entrypoint','/bin/sh',p_image['Id'],'-c',
            'chmod 700 /config /data; chmod -R u+rwX,go-rwx /config; chmod -R a+rX /operator; mkdir -p /data/runtime; /usr/local/bin/rx-platformd init /config/startup.json && chown -R 10001:10001 /data /config')
        run('docker','cp',str(fixture)+'/.',names['setup']+':/config')
        run('docker','cp',str(bundle)+'/.',names['setup']+':/operator')
        run('docker','start','--attach',names['setup']);assert state(names['setup'])['State']['ExitCode']==0
        run('docker','run','-d','--name',names['service'],'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw',
            '-v',names['config']+':/config:ro','-v',names['data']+':/data:rw','-v',names['assets']+':/operator:ro',
            '-p',f'127.0.0.1:{port}:8443',p_image['Id'],'run','/config/startup.json')
        tls=ssl.create_default_context(cafile=str(fixture/browser_data['ca']))
        tls.load_cert_chain(str(fixture/browser_data['certificate']),str(fixture/browser_data['private_key']))
        opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPSHandler(context=tls))
        deadline=time.monotonic()+25
        while True:
            assert state(names['service'])['State']['Running'],run('docker','logs',names['service'])
            try:
                with opener.open(origin+'/',timeout=2) as response:
                    assert response.status==200 and response.read()==(bundle/'index.html').read_bytes()
                    assert "script-src 'self'" in response.headers['content-security-policy']
                break
            except (OSError,urllib.error.URLError):
                if time.monotonic()>deadline:raise
                time.sleep(.1)
        with opener.open(urllib.request.Request(origin+'/',method='HEAD'),timeout=3) as response:
            assert response.read()==b'' and int(response.headers['content-length'])==(bundle/'index.html').stat().st_size
        no_cert=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(fixture/browser_data['ca']))))
        try:
            no_cert.open(origin+'/',timeout=3)
            raise AssertionError('UI accepted a client without a certificate')
        except (ssl.SSLError,urllib.error.URLError,OSError):pass
        with sync_playwright() as playwright:
            browser=playwright.chromium.launch(headless=True)
            # Test CA was verified above using Python TLS. Browser fixtures bypass only local test-CA trust.
            context=browser.new_context(ignore_https_errors=True,viewport={'width':1440,'height':1100},client_certificates=[{'origin':origin,'certPath':str(fixture/browser_data['certificate']),'keyPath':str(fixture/browser_data['private_key'])}])
            page=context.new_page();errors=[];csp=[];external=[]
            page.on('pageerror',lambda e:errors.append(str(e)))
            page.on('console',lambda m:csp.append(m.text[:350]) if 'Content Security Policy' in m.text or 'Refused to' in m.text else None)
            page.on('request',lambda r:external.append(r.url) if not r.url.startswith(origin+'/') else None)
            page.goto(origin+'/',wait_until='networkidle')
            expect(page.get_by_role('heading',name='운영 공간에 로그인')).to_be_visible()
            page.get_by_label('계정',exact=True).fill(browser_data['principal'])
            page.get_by_label('비밀번호',exact=True).fill(browser_data['password'])
            page.get_by_role('button',name='로그인',exact=True).click()
            expect(page.get_by_role('button',name='로그아웃',exact=True)).to_be_visible()
            page.evaluate('document.fonts.ready')
            assert page.evaluate('(font) => document.fonts.check(font, "운영 조건 확인")', '400 16px "IBM Plex Sans KR"')
            overview=context.request.get(origin+'/api/v1/overview').json()
            assert overview['user']['terminal']==browser_data['terminal']
            assert all(not c['runs'] and c['cell']['value']['qualification'] is None for c in overview['cells'])
            for path in ['/assets/missing.js','/api/v1/missing','/unregistered-ui-route']:
                response=context.request.get(origin+path);assert response.status==404 and 'application/json' in response.headers['content-type']
            assert context.request.post(origin+'/',data={}).status==405
            page.screenshot(path=str(out/'operator-desktop.png'),full_page=True)
            sent=[]
            def lose_reply(route):
                sent.append(route.request.post_data_json)
                response=route.fetch();assert response.ok,response.text()
                route.abort('failed')
            page.route('**/api/v1/runs',lose_reply,times=1)
            page.get_by_role('button',name='새 실행 준비').click()
            page.get_by_role('button',name='실행 기록 만들기',exact=True).click()
            expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_be_visible()
            page.reload(wait_until='networkidle')
            expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_be_visible()
            recovered=[]
            def recover_reply(route):
                recovered.append(route.request.post_data_json);route.continue_()
            page.route('**/api/v1/runs',recover_reply,times=1)
            page.get_by_role('button',name='같은 요청 확인',exact=True).click()
            expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_have_count(0)
            assert sent==recovered and len(sent)==1
            after=context.request.get(origin+'/api/v1/overview').json()
            assert sum(len(c['runs']) for c in after['cells'])==1
            assert all(c['cell']['value']['qualification'] is None for c in after['cells'])
            page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(out/'operator-mobile.png'),full_page=True)
            assert page.evaluate('document.documentElement.scrollWidth <= window.innerWidth')
            assert not errors and not csp and not external,(errors,csp,external)
            wrong=browser.new_context(ignore_https_errors=True,client_certificates=[{'origin':origin,'certPath':str(fixture/'probe.pem'),'keyPath':str(fixture/'probe.key')}])
            bad=wrong.request.post(origin+'/api/v1/session',data={'principal':browser_data['principal'],'password':browser_data['password']},headers={'Origin':origin,'X-RX-Client':'browser-v1'})
            assert bad.status==403
            wrong.close();context.close();browser.close()
        running=state(names['service']);assert running['Config']['User']=='10001:10001' and running['HostConfig']['ReadonlyRootfs']
        assert running['HostConfig']['CapDrop']==['ALL']
        assert any(m['Destination']=='/operator' and not m['RW'] for m in running['Mounts'])
        run('docker','stop','--time','15',names['service']);assert state(names['service'])['State']['ExitCode']==0
        result={'schema':'rx.operator-delivery-test.v1','status':'PASS','platform_image':p_image['Id'],'solutions_image':s_image['Id'],'bundle_manifest_sha256':pin,'bundle_files':len(manifest['files']),
                'checks':['UI extracted from pinned S image','fresh P installation and non-root read-only runtime','same-origin direct registered-terminal mTLS login','verified server TLS and missing-client-certificate denial','real production bundle and Korean fonts; no external assets/dev server','API/asset/deep-link 404 and static method restrictions','CreateRun committed reply loss and identical request recovery after reload','one Run without qualification or native execution','unregistered service certificate cannot log in','no JS/CSP errors or mobile overflow','graceful P software stop'],
                'limitations':['Browser ignores trust errors for the disposable test CA; Python TLS separately verifies the server certificate.','CreateRun is not StartRun/executor assignment.','S image was used as immutable asset source; its Host/controller was not started.','No physical equipment or qualification; disposable test identities/volumes only.']}
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','bundle':pin,'files':len(manifest['files'])}))
    except Exception:
        try:(out/'platform.log').write_text(run('docker','logs',names['service']))
        except Exception:pass
        raise
    finally:
        for key in ['service','setup','source']:subprocess.run(['docker','rm','-f',names[key]],capture_output=True)
        for key in ['config','data','assets']:subprocess.run(['docker','volume','rm',names[key]],capture_output=True)
