"""Registered-terminal browser start and independent file-device outcome checks."""
from __future__ import annotations
import json
import time
from playwright.sync_api import sync_playwright, expect
from .api import publish_new
from .workflow import main_cell


def exercise_operator(c:dict, active:dict)->None:
    docker=c['docker'];browser=c['browser'];cell=c['delivery']['cell'];origin=browser['origin']
    observer=c['users']['engineer'];negative=c['delivery']['negative_cell']
    before=observer.get('/api/v1/overview')
    assert not main_cell(before,cell)['runs']
    assert main_cell(before,cell)['cell']['value']['commissioning']=='COMMISSIONED'
    assert main_cell(before,negative)['cell']['value']['commissioning']=='NOT_COMMISSIONED'
    with sync_playwright() as p:
        engine=p.chromium.launch(headless=True)
        context=engine.new_context(ignore_https_errors=True,viewport={'width':1440,'height':1100},client_certificates=[{
            'origin':origin,'certPath':str(c['final']/browser['certificate']),'keyPath':str(c['final']/browser['private_key'])}])
        page=context.new_page();errors=[];csp=[]
        page.on('pageerror',lambda error:errors.append(str(error)))
        page.on('console',lambda message:csp.append(message.text) if 'Content Security Policy' in message.text or 'Refused to' in message.text else None)
        page.goto(origin,wait_until='networkidle')
        page.get_by_label('Account',exact=True).fill('operator')
        page.get_by_label('Password',exact=True).fill(browser['credentials']['operator'])
        page.get_by_role('button',name='Sign in',exact=True).click()
        expect(page.get_by_role('button',name='Sign out',exact=True)).to_be_visible()
        page.get_by_role('button',name='Prepare new run').click()
        page.get_by_role('button',name='Create run record',exact=True).click()
        deadline=time.monotonic()+20
        while True:
            current=observer.get('/api/v1/overview');runs=main_cell(current,cell)['runs']
            if runs:break
            if time.monotonic()>deadline:raise AssertionError('UI did not create an execution record')
            page.wait_for_timeout(100)
        assert len(runs)==1
        run=runs[0]['value']['id']
        page.get_by_role('combobox').filter(has=page.locator(f'option[value="{run}"]')).select_option(run)
        page.get_by_label('Material attempt count',exact=True).fill('2')
        button=page.get_by_role('button',name='Review start details',exact=True)
        try:
            expect(button).to_be_enabled(timeout=20000)
        except Exception:
            page.screenshot(path=str(docker.evidence/'operator-start-unavailable.png'),full_page=True)
            publish_new(docker.evidence/'start-unavailable-overview.json',observer.get('/api/v1/overview'))
            publish_new(docker.evidence/'start-unavailable-context.json',c['users']['operator'].get('/api/v1/run/start-context',cell=cell,run=run,purpose='PRODUCTION',budget_limit='2'))
            raise
        button.click()
        expect(page.get_by_role('heading',name='Start this run?')).to_be_visible()
        page.screenshot(path=str(docker.evidence/'operator-start-confirmation.png'),full_page=True)
        sent=[]
        def lose_reply(route):
            sent.append(route.request.post_data_json)
            response=route.fetch();assert response.ok,response.text()
            route.abort('failed')
        page.route('**/api/v1/runs/start',lose_reply,times=1)
        page.get_by_role('button',name='Request start with reviewed count',exact=True).click()
        expect(page.get_by_role('button',name='Check original request',exact=True)).to_be_visible()
        page.reload(wait_until='networkidle')
        expect(page.get_by_role('button',name='Check original request',exact=True)).to_be_visible()
        recovered=[]
        def recover_reply(route):
            recovered.append(route.request.post_data_json);route.continue_()
        page.route('**/api/v1/runs/start',recover_reply,times=1)
        page.get_by_role('button',name='Check original request',exact=True).click()
        expect(page.get_by_role('button',name='Check original request',exact=True)).to_have_count(0,timeout=20000)
        assert len(sent)==1 and sent==recovered
        publish_new(docker.evidence/'ui-start-request-recovery.json',{'sent':sent,'recovered':recovered})
        deadline=time.monotonic()+45
        while True:
            for name in {c['p'],c['h'],c['e']}:
                assert docker.state(name)['State']['Running'],f'product process stopped before work completion: {name}'
            current=observer.get('/api/v1/overview');scope=main_cell(current,cell)
            value=next(item['value'] for item in scope['runs'] if item['value']['id']==run)
            if value['state']=='COMPLETED':break
            if time.monotonic()>deadline:raise AssertionError(f'product Run did not complete: {value}')
            page.wait_for_timeout(100)
        assert len(value['part_ids'])==2 and len(set(value['part_ids']))==2
        assert value['budget']['limit']=='2' and len(value['budget']['consumptions'])==2
        work=[item for item in scope['work'] if item['run']==run]
        assert len(work)==2
        assert all(item['operation']['outcome']=='SUCCEEDED' and item['operation']['disposition']=='RELEASED' for item in work)
        raw=docker.run('exec',c['h'],'cat','/data/host/device/effects.jsonl')
        effects=[json.loads(line) for line in raw.splitlines() if line]
        assert len(effects)==2 and len({v['invocation'] for v in effects})==2
        assert {v['operation'] for v in effects}=={v['operation']['operation_id'] for v in work}
        assert all(v['capture']['captured_at']['clock_id']==c['installation']['clock_id'] for v in effects)
        physical=main_cell(current,negative)['cell']['value']
        assert physical['commissioning']=='NOT_COMMISSIONED' and physical['qualification'] is None
        expect(page.get_by_role('region',name='Selected run start and status').get_by_text('Current run state · Attempt completed',exact=True)).to_be_visible(timeout=15000)
        page.screenshot(path=str(docker.evidence/'operator-completed.png'),full_page=True)
        page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(docker.evidence/'operator-completed-mobile.png'),full_page=True)
        assert page.evaluate('document.documentElement.scrollWidth <= window.innerWidth')
        assert not errors and not csp,(errors,csp)
        context.close();engine.close()
    # These are normal stops of isolated file-device processes; final cleanup is separate.
    supervisor_stop=None
    if c['composition']=='supervisor':
        from .supervisor import stop
        supervisor_stop=stop(c)
    else:
        docker.run('stop','--time','15',c['e']);assert docker.state(c['e'])['State']['ExitCode']==0
        docker.run('stop','--time','15',c['h']);assert docker.state(c['h'])['State']['ExitCode']==0
    stop_file=c['materials'].temporary/'host-stop.json'
    docker.run('cp',c['h']+':/run/rx-host/host-status.json',str(stop_file))
    host_stop=json.loads(stop_file.read_text())
    assert host_stop['phase'] in ('STOPPED','STOPPED_WITH_RECONCILIATION_REQUIRED')
    assert host_stop['stop']['safe_to_drop'] and not host_stop['stop']['physical_shutdown_assessed']
    docker.run('stop','--time','15',c['p']);p_exit=docker.state(c['p'])['State']['ExitCode']
    assert p_exit in (0,2), 'P software stop failed'
    publish_new(docker.evidence/'result.json',{
        'schema':'rx.product-cell-delivery-test.v1','status':'PASS','platform_image':c['p_image']['Id'],'solutions_image':c['s_image']['Id'],
        'container_count':2 if supervisor_stop else 3,'composition':c['composition'],
        'supervisor_stop':supervisor_stop,'simulation_only':True,'runtime_clock':c['installation']['clock_id'],
        'activation':active['id'],'run':value,'work':work,'independent_native_effects':effects,
        'physical_control':physical,'same_start_request_recovered':True,'p_stop_exit':p_exit,'host_stop':host_stop,
        'limitations':['FILE_SIMULATION only; no physical equipment or field qualification.',
                       'Supervised Host/Executor composition assessed only for this file-device cell.' if supervisor_stop else 'Two product image types with independent P/Host/Executor containers; supervisor composition is assessed separately.',
                       'P stop may retain explicit negative-control/Host fence attention; no full field shutdown claim.',
                       'Browser bypasses disposable test CA trust only; Python terminal TLS independently verifies CA/leaf.']})
