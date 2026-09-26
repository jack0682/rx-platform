"""Measured selected SIMULATION commissioning, followed by registered-terminal product UI execution."""
from __future__ import annotations
import hashlib
import json
import shutil
from pathlib import Path
from .api import Rejected, encoded, publish_new
from .commission import Commission, wait_for
from .qualification import build_report, verified_release_evidence


def main_cell(overview:dict, cell:str)->dict:
    return next(value for value in overview['cells'] if value['cell']['value']['id']==cell)


def run(c:dict)->None:
    users=c['users'];engineer=users['engineer'];reviewer=users['verifier'];release=users['release'];operator=users['operator']
    delivery=c['delivery'];cell=delivery['cell'];docker=c['docker'];materials=c['materials'];s=c['s_image']['Id']
    def overview():return engineer.get('/api/v1/overview')
    connected=wait_for(overview,lambda v:all(h['context']=='CURRENT' for h in main_cell(v,cell)['diagnostics']['hosts']) and len(main_cell(v,cell)['diagnostics']['hosts'])>0 and all(s['usable'] for s in main_cell(v,cell)['diagnostics']['sources']))
    assert not main_cell(connected,cell)['runs']
    host=json.loads(docker.run('exec',c['h'],'cat','/run/rx-host/host-status.json'))
    assert host['phase']=='SOFTWARE_READY_UNARMED' and host['installation']==c['installation']['id']
    assert host['clock_id']==c['installation']['clock_id']
    assert not host['qualification_or_arm_restored']
    original_journals=json.loads(docker.run('exec',c['h'],'cat','/data/host/installation.json'))
    release_proof=verified_release_evidence(c['release_evidence'],s)
    # Deliberately unconfigured independent PHYSICAL cell; no endpoint, Host or device attached.
    negative=operator.get('/api/v1/cell',id=delivery['negative_cell'])
    assert negative['value']['configuration']['environment']=='PHYSICAL'
    assert negative['value']['commissioning']=='NOT_COMMISSIONED'
    cfg=negative['value']['configuration']
    rejected_run=operator.mutate('negative-create','/api/v1/runs',{'cell':cfg['id'],'recipe_digest':cfg['recipe']['sha256'],'site_config_digest':cfg['site_config_digest'],'expected_cell':negative['revision']})
    candidate=operator.get('/api/v1/run/start-context',cell=cfg['id'],run=rejected_run['id'],purpose='PRODUCTION',budget_limit='2')
    assert not candidate['can_request'] and candidate['blocking_reason']=='NOT_COMMISSIONED'
    try:operator.mutate('negative-start','/api/v1/runs/start',candidate['request'])
    except Rejected as rejection:
        assert rejection.status in (403,409,422)
        negative_denial={'status':rejection.status,'body':rejection.body,'candidate':candidate}
    else:raise AssertionError('unconfigured PHYSICAL start was accepted')
    try:operator.get('/api/v1/package-intake-context',cell=cell)
    except Rejected as rejection:
        assert rejection.status==403
        role_denial={'status':rejection.status,'body':rejection.body}
    else:raise AssertionError('Operator unexpectedly accessed Engineer intake context')
    commission=Commission(engineer,reviewer,release,cell,c['target'])
    package=delivery['package'];job=commission.import_process(package['manifest'],package['signature'],delivery['binding_selections'])
    first=engineer.recover('process-intake');second=engineer.recover('process-intake')
    assert encoded(first)==encoded(second)
    request_dir=materials.temporary/'review-input';request_dir.mkdir()
    publish_new(request_dir/'process-review-request.json',job['request'])
    docker.put(s,materials.compiler_work,request_dir)
    mounts=[materials.compiler_config+':/config:ro',materials.compiler_work+':/work:rw']
    result=json.loads(docker.command(s,'/opt/rx/bin/rx-process-package',[
        'review','/work/package','/config/package-policy.json','/work/process-review-request.json','/work/process-review'],mounts,'process-review'))
    assert result['compiler_checks_passed'] and not result['activation_authorized']
    authority=json.loads((c['final']/'config/review-authority.json').read_text());key=authority['keys'][0]['id']
    docker.command(s,'/opt/rx/bin/rx-process-package',['review-signing-request','/work/process-review/verification.json',key,'/work/review-signing.json'],mounts,'review-signing-request')
    signing=materials.temporary/'review-signing.json';docker.extract(s,materials.compiler_work,'review-signing.json',signing)
    raw=json.loads(signing.read_text());sig=materials.sign({'key':raw['key'],'message_hex':raw['message_hex']},'review')
    review=materials.temporary/'process-review';docker.extract(s,materials.compiler_work,'process-review',review)
    shutil.copyfile(sig,review/'verification.sig.json');(review/'verification.sig.json').chmod(0o644)
    materials.preserve_public(review, 'process-review')
    materials.preserve_public(c['final']/'config/review-authority.json', 'review-authority')
    publication=materials.temporary/'review-publication';publication.mkdir();shutil.copytree(review,publication/'process-review')
    docker.put(c['p_image']['Id'],c['imports'],publication)
    applied=commission.accept_process_report(job,result['report_digest'])
    current=overview();assert not main_cell(current,cell)['runs'] and main_cell(current,cell)['cell']['value']['qualification'] is None
    qjob=commission.begin_requalification(applied,delivery['qualification_policy_digest'])
    qdetail=release.get('/api/v1/qualification-review',cell=cell,id=qjob['request']['id'])
    assert qdetail['context_current'] and qdetail['fences_confirmed']
    equipment=main_cell(overview(),cell)['diagnostics']
    current_journals=json.loads(docker.run('exec',c['h'],'cat','/data/host/installation.json'))
    assert current_journals==original_journals
    if 'observe_native_effects' in c:
        native_observation=c['observe_native_effects']()
        effects=json.dumps(native_observation) if native_observation else ''
    else:
        effects=docker.run('exec',c['h'],'/bin/sh','-c','if [ -f /data/host/device/effects.jsonl ]; then cat /data/host/device/effects.jsonl; fi')
    assert effects.strip()==''
    assert not main_cell(overview(),cell)['runs']
    observations={
        'SOFTWARE':{'assertions':{'actual_signed_compiler_passed':result['compiler_checks_passed'],'actual_target_matches_policy':applied['change']['after']['sha256']==delivery['target_configuration_digest']},'observations':{'compiler':result,'process_review':job,'applied_configuration':applied['change']['after']},'limitations':['Selected software simulation process only; no physical validation.']},
        'EQUIPMENT':{'assertions':{'selected_simulation_current_sources':bool(equipment['sources']) and all(v['usable'] for v in equipment['sources']),'host_service_ready_unarmed':host['phase']=='SOFTWARE_READY_UNARMED','same_kernel_clock':host['clock_id']==c['installation']['clock_id'],'unchanged_durable_host_journals':current_journals==original_journals},'observations':{'host':host,'diagnostics':equipment,'installation_journals':current_journals},'limitations':['Simulated native health is not a physical interlock or mechanical qualification.', *c.get('host_fixture_limitations',[])]},
        'CELL_INTEGRATION':{'assertions':{'configuration_applied_unqualified':applied['change']['state']=='APPLIED_UNQUALIFIED','host_metadata_acknowledged_before_apply':commission.pre_apply_detail['host_configuration']['all_hosts_acknowledged'],'committed_host_proofs_present':bool(applied['change']['application']['host_proofs']),'actual_requalification_fences_confirmed':qdetail['fences_confirmed']},'observations':{'change':applied['change'],'host_configuration_before_apply':commission.pre_apply_detail['host_configuration'],'committed_host_proofs':applied['change']['application']['host_proofs'],'fences':qjob['request']['fences'],'reviewed_clearance_snapshot':commission.clearance_snapshot,'proposed_clear_block_ids':commission.clear_candidates},'limitations':['No production Run exists yet; actual material cycle is tested after explicit activation.']},
        'RECOVERY':{'assertions':{'sealed_release_recovery_tests_verified':len(release_proof['verified_tests'])==3,'host_journal_identity_continuous':current_journals==original_journals,'same_original_intake_request_recovers_exact_receipt':encoded(first)==encoded(second),'runtime_restrictions_have_exact_recorded_origins':bool(qjob['request']['runtime_restrictions'])},'observations':{'sealed_release_evidence':release_proof,'original_and_recovered_receipt':first,'runtime_origins':qjob['request']['runtime_restrictions']},'limitations':['Receipt recovery and restart restriction provenance only; native UNKNOWN recovery and process restart rebind are not certified.']},
        'PROTECTION':{'assertions':{'unconfigured_physical_start_rejected':candidate['blocking_reason']=='NOT_COMMISSIONED','read_did_not_create_start_permission':candidate['can_request'] is False},'observations':negative_denial,'limitations':['Software admission boundary only. Physical protective devices are unconfigured and not assessed.']},
        'OPERATIONS':{'assertions':{'operator_cannot_submit_engineer_intake':role_denial['status']==403,'separate_human_roles':len({engineer.get('/api/v1/overview')['user']['principal'],reviewer.get('/api/v1/overview')['user']['principal'],release.get('/api/v1/overview')['user']['principal']})==3,'main_run_count_zero_before_qualification':not main_cell(current,cell)['runs'],'native_effects_zero_before_qualification':effects.strip()==''},'observations':{'role_denial':role_denial,'terminal':c['browser']['terminal']},'limitations':['Registered-terminal/API commissioning controls; operator motion UI is exercised after activation.']},
    }
    qreport=materials.temporary/'qualification-report'
    report=build_report(qjob,c['validator'],c['final']/'qualification-materials',observations,qreport,scope=c.get('native_scope','FILE_SIMULATION_DELIVERY_ONLY'))
    qpolicy=json.loads((c['final']/'config/qualification-policy.json').read_text());qkey=qpolicy['keys'][0]['id']
    signature=materials.sign({'key':qkey,'qualification_report':str(qreport/'qualification.json')},'qualification')
    metadata=json.loads(signature.with_suffix('.metadata.json').read_text())
    shutil.copyfile(signature,qreport/'qualification.sig.json');(qreport/'qualification.sig.json').chmod(0o644)
    materials.preserve_public(qreport, 'qualification-report')
    materials.preserve_public(c['final']/'config/qualification-policy.json', 'qualification-policy')
    publication=materials.temporary/'qualification-publication';publication.mkdir();shutil.copytree(qreport,publication/'qualification-report')
    docker.put(c['p_image']['Id'],c['imports'],publication)
    active=commission.activate(qjob,metadata['report_digest'])
    publish_new(docker.evidence/'commissioning.json',{'schema':'rx.delivery-commissioning.v1','status':'PASS','activation':active,'simulation_only':True,'private_signers_mounted':False})
    if c.get('after_commissioning') is not None:
        c['after_commissioning'](c,active)
    else:
        from .operator import exercise_operator
        exercise_operator(c,active)
