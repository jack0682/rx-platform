#!/usr/bin/env python3
"""Real terminal approval and original Fence recovery on top of the idle P/H restart test."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import test_host_reconnect as restart
from cell_delivery.api import Api, Rejected, encoded, publish_new


def _login(c, account, label):
    browser=c['browser'];final=c['final']
    api=Api(browser['origin'],final/browser['ca'],final/browser['certificate'],final/browser['private_key'],c['docker'].evidence/'api'/label)
    api.login(account,browser['credentials'][account]);return api


def _host_grants(c,label):
    script="""import json,sqlite3
d=sqlite3.connect('file:/data/host/host.db?mode=ro',uri=True);d.execute('PRAGMA query_only=ON');d.execute('BEGIN')
r=[{'key':k,'revision':v,'value':json.loads(b)} for k,v,b in d.execute("SELECT key,revision,document FROM entities WHERE key LIKE 'grant/%' OR key LIKE 'grant-request/%' ORDER BY key")]
d.rollback();d.close();print(json.dumps(r,sort_keys=True,separators=(',',':')))
"""
    return json.loads(c['docker'].command(c['s_image']['Id'],'/usr/bin/python3',['-c',script],[c['h_data']+':/data:ro'],label))


def exercise(c):
    docker=c['docker'];before=c['stable'];host='host/sim';cell=c['delivery']['cell']
    assert len(before['baselines'])==1, 'actual pinned initial binding must record its provenance'
    assert before['baselines']==c['initial']['baselines'], 'restart must preserve original baseline bytes'
    baseline=before['baselines'][0]['value']
    assert baseline['host_boot']==before['producer']['value']['peer_boot']
    assert baseline['producer_session']!=before['producer']['value']['session']
    assert baseline['producer_authentication_binding']==before['producer']['value']['authentication_binding']
    assert baseline['source']['size_bytes']!='0' and baseline['transport_digest']
    grants=_host_grants(c,'recovery-grants-before')
    release=_login(c,'release','recovery-release')
    operator=_login(c,'operator','recovery-operator')
    try:operator.get('/api/v1/host-recovery-context',host=host,origin=cell)
    except Rejected as error:assert error.status==403
    else:raise AssertionError('Operator accessed ReleaseManager recovery context')
    context=release.get('/api/v1/host-recovery-context',host=host,origin=cell)
    assert not context['context']['blockers'],context['context']['blockers']
    assert context['context']['producer']['session']==before['producer']['value']['session']
    proposal=release.mutate('host-recovery-propose','/api/v1/host-recoveries',{
        'host':host,'origin':cell,'expected_context':context['context_digest'],'expected_cells':context['expected_cells']})
    binding=proposal['view']['binding'];rid=binding['id']
    assert binding['phase']=='PROPOSED' and proposal['view']['operation_authorized'] is False
    assert binding['requested_context_digest']==context['context_digest']
    # Simulate an ignored/lost HTTP result by recovering the exact stored request.
    recovered=release.recover('host-recovery-propose')
    assert recovered['view']['binding']['id']==rid and recovered['proposal_digest']==proposal['proposal_digest']
    listed=release.get('/api/v1/host-recoveries',host=host,limit=50)
    assert any(row['view']['binding']['id']==rid for row in listed['items'])
    tasks=binding['fences'];assert set(tasks)=={cell}
    expected=next(row for row in before['outbox'] if row['id']==tasks[cell]['task']['request'])
    assert tasks[cell]['task']['originating_message']==expected['id']
    assert expected['state'] in ('NEW','EMIT_ENTERED') and expected['value']['kind']=='FENCE'
    assert _host_grants(c,'recovery-grants-proposed')==grants
    assert c['oracle']()['outbox']==before['outbox'], 'proposal must not emit the Fence'
    approved=release.mutate('host-recovery-approve','/api/v1/host-recovery/approve',{
        'id':rid,'expected_revision':binding['revision'],'proposal_digest':proposal['proposal_digest'],
        'expected_cells':context['expected_cells']})
    assert approved['view']['binding']['phase']=='RECOVERY_ONLY',approved
    assert approved['view']['operation_authorized'] is False
    assert approved['proposal_digest']==proposal['proposal_digest']
    for name,step in tasks.items():
        final_step=approved['view']['binding']['fences'][name]
        assert final_step['task']==step['task'] and final_step['payload_digest']==step['payload_digest']
        assert final_step['phase']=='ACKNOWLEDGED'
        assert final_step['acknowledgment']['invalidation']==expected['id']
    approved_again=release.recover('host-recovery-approve')
    assert approved_again['view']['binding']['id']==rid and approved_again['proposal_digest']==proposal['proposal_digest']
    assert approved_again['view']['binding']['fences']==approved['view']['binding']['fences']
    current=c['oracle']()
    assert current['hosts']==before['hosts'] and current['baselines']==before['baselines']
    for original in before['outbox']:
        now=next(row for row in current['outbox'] if row['id']==original['id'])
        assert now['value']==original['value']
        assert now['state']==('DELIVERED' if original['id']==expected['id'] else original['state'])
    assert all(row['value']['kind']=='FENCE' for row in current['outbox'])
    assert _host_grants(c,'recovery-grants-after')==grants
    assert current['cells'][0]['value']['blocks']==before['cells'][0]['value']['blocks']
    assert c['evidence_count']('recovery-evidence-after')==0
    effects=docker.run('exec',c['h'],'/bin/sh','-c','if [ -f /data/host/device/effects.jsonl ]; then cat /data/host/device/effects.jsonl; fi')
    assert not effects.strip()
    # An unapproved operation ID cannot use this binding as a generic Host command channel.
    try:release._request('/api/v1/host-recovery/query',encoded({'id':rid,'operation':rid}))
    except Rejected as error:assert error.status in (403,409,422)
    else:raise AssertionError('arbitrary operation query was accepted')
    refreshed=release._request('/api/v1/host-recovery/progress',encoded({'id':rid}))
    assert refreshed['view']['binding']['phase']=='RECOVERY_ONLY' and not refreshed['view']['operation_authorized']
    value={'schema':'rx.host-recovery-product-test.v1','status':'PASS','simulation_only':True,
        'binding':rid,'proposal_digest':proposal['proposal_digest'],'requested_context_digest':context['context_digest'],
        'original_fence':expected,'approved':approved,'after':current,'baseline':baseline,
        'grant_records_unchanged':True,'operating_registration_unchanged':True,'restrictions_preserved':True,
        'native_effects':0,'new_evidence':0,'scope':'Explicit recovery-only communication; no operating rebind, qualification, Arm or resume.',
        'platform_image':c['p_image']['Id'],'solutions_image':c['s_image']['Id']}
    publish_new(docker.evidence/'recovery-result.json',value)
    return {'result':'recovery-result.json','sha256':hashlib.sha256((docker.evidence/'recovery-result.json').read_bytes()).hexdigest()}


if __name__=='__main__':restart.main(exercise)
