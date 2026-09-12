"""Public API commissioning sequence. Every receipt/Host acknowledgement comes from P."""
from __future__ import annotations
import hashlib
import time
import uuid
from .api import Api, encoded, publish_new


def uid() -> str: return str(uuid.uuid4())

def transition(change: dict) -> dict:
    return {'change':change['id'],'cell':change['cell'],'expected':change['revision'],'plan_digest':change['plan_digest']}


def wait_for(read, accepted, *, timeout: float = 45):
    deadline=time.monotonic()+timeout
    last=None
    while time.monotonic()<deadline:
        last=read()
        if accepted(last):return last
        time.sleep(.25)
    raise RuntimeError(f'public state did not reach required condition: {str(last)[:2400]}')


class Commission:
    def __init__(self, engineer:Api, reviewer:Api, release:Api, cell:str, target:dict):
        self.engineer,self.reviewer,self.release=engineer,reviewer,release
        self.cell,self.target=cell,target
        self.target_digest=hashlib.sha256(encoded(target)).hexdigest()

    def import_process(self, manifest:str, signature:str, binding_selections:dict)->dict:
        api=self.engineer
        context=api.get('/api/v1/package-intake-context',cell=self.cell)
        package=api.mutate('process-intake','/api/v1/package-intakes',{
            'id':uid(),'cell':self.cell,'title':'Signed FILE_SIMULATION delivery process',
            'relative_path':'package','object':{'manifest':manifest,'signature':signature},
            'configuration_digest':context['configuration_digest'],
            'policy_generation':context['registration']['generation']})
        return api.mutate('process-review','/api/v1/process-reviews',{
            'id':uid(),'intake':package['id'],'cell':self.cell,
            'configuration_digest':context['configuration_digest'],
            'policy_generation':context['registration']['generation'],
            'binding_selections':binding_selections,'device_plans':[]})

    def accept_process_report(self, job:dict, report_digest:str)->dict:
        version=self.engineer.mutate('process-report','/api/v1/process-review/reports',{
            'review':job['request']['id'],'cell':self.cell,'expected':None,
            'directory':'process-review','report_digest':report_digest})
        if not version['ready_for_software_approval']:
            raise RuntimeError('actual compiler report is not ready for independent approval')
        decision=self.reviewer.mutate('process-approve','/api/v1/process-review/decisions',{
            'review':job['request']['id'],'cell':self.cell,'report_revision':version['revision'],
            'review_digest':version['review_digest'],'expected':None,'choice':'APPROVE',
            'note':'Reviewed actual signed package/compiler results for FILE_SIMULATION only.'})
        change=self.engineer.mutate('process-change','/api/v1/process-changes',{
            'id':uid(),'cell':self.cell,'mode':'REPLACE',
            'review':{'id':job['request']['id'],'revision':version['revision'],
                      'review_digest':version['review_digest'],'decision_revision':decision['revision']},
            'reason':'First signed simulation process installation; qualification remains separate.'})
        if change['after']['sha256'] != self.target_digest:
            raise RuntimeError('actual P proposed target differs from pinned pre-installation policy target')
        change=self.reviewer.mutate('process-impact','/api/v1/process-change/impact-review',{
            'target':transition(change),'note':'Checked the exact isolated simulation cell and Host/resource impact.'})
        change=self.release.mutate('process-stage','/api/v1/process-change/stage',transition(change))
        change=self.release.mutate('process-prepare','/api/v1/process-change/prepare',{
            'target':transition(change),'refresh':False})
        change_id=change['id']
        def read():return self.release.get('/api/v1/process-change',cell=self.cell,id=change_id)
        detail=wait_for(read,lambda v: not any(b['kind']=='HOST_FENCE_UNCONFIRMED' for b in v['blockers']))
        if detail['change'].get('host_binding_plan') is not None:
            raise RuntimeError('native Host binding replacement remains a separate procedure')
        self.release.mutate('process-hosts','/api/v1/process-change/configure-hosts',transition(detail['change']))
        detail=wait_for(read,lambda v:v['host_configuration']['all_hosts_acknowledged'] and not v['host_configuration']['outcome_unknown'] and not v['host_configuration']['mixed_configuration'])
        self.pre_apply_detail=detail
        applied=self.release.mutate('process-apply','/api/v1/process-change/apply',transition(detail['change']))
        if applied['state'] != 'APPLIED_UNQUALIFIED':raise RuntimeError('first process must remain unqualified after application')
        current=self.release.get('/api/v1/process-change',cell=self.cell,id=applied['id'])
        if encoded(current['after']) != encoded(self.target):raise RuntimeError('P applied configuration differs from declared target')
        return current

    def begin_requalification(self, applied:dict, policy_digest:str)->dict:
        restrictions=self.release.get('/api/v1/runtime-restrictions',cell=self.cell)
        origins={r['block']['id']:r['origin_digest'] for r in restrictions['restrictions']}
        if not origins or any(value is None for value in origins.values()):
            raise RuntimeError('initial RuntimeRestart requires actual recorded origins')
        cell=self.release.get('/api/v1/cell',id=self.cell)
        job=self.release.mutate('qualification-review','/api/v1/qualification-reviews',{
            'id':uid(),'change':applied['change']['id'],'cell':self.cell,
            'expected_change':applied['change']['revision'],
            'expected_cells':{self.cell:cell['revision']},'policy_digest':policy_digest,
            'runtime_restrictions':origins})
        wait_for(lambda:self.release.get('/api/v1/qualification-review',cell=self.cell,id=job['request']['id']),
                 lambda v:v['context_current'] and v['fences_confirmed'])
        self.clearance_snapshot=self.release.get('/api/v1/cell',id=self.cell)
        blocks=self.clearance_snapshot['value']['blocks']
        if any(b['reason'] not in ('CONFIGURATION_CHANGE','RUNTIME_RESTART') for b in blocks):
            raise RuntimeError('fresh installation contains an unrelated restriction requiring separate resolution')
        self.clear_candidates=sorted(b['id'] for b in blocks)
        selected=next(c for c in job['request']['cells'] if c['profile']['cell']==self.cell)
        if not set(selected['blocks']).issubset(self.clear_candidates):
            raise RuntimeError('signed requalification restrictions disappeared')
        return job

    def activate(self, job:dict, external_report_digest:str)->dict:
        version=self.engineer.mutate('qualification-report','/api/v1/qualification-review/reports',{
            'review':job['request']['id'],'cell':self.cell,'expected':None,
            'directory':'qualification-report','report_digest':external_report_digest})
        if not version['ready_for_review']:raise RuntimeError('six-area evidence is not ready for independent review')
        decision=self.reviewer.mutate('qualification-approve','/api/v1/qualification-review/decisions',{
            'review':job['request']['id'],'cell':self.cell,'report_revision':version['revision'],
            'report_digest':version['digest'],'expected':None,'choice':'APPROVE',
            'note':'Reviewed six separate simulation acceptance criteria and their scoped evidence.'})
        cell=self.release.get('/api/v1/cell',id=self.cell)
        # The report evidence includes this exact proposed list. P separately verifies
        # each block owner against this Change/Job; a reason name is never sufficient.
        if sorted(b['id'] for b in cell['value']['blocks'])!=self.clear_candidates:
            raise RuntimeError('restriction set changed after independent review')
        batch=self.release.mutate('qualification-issue','/api/v1/qualification-activations',{
            'review':job['request']['id'],'cell':self.cell,'report_revision':version['revision'],
            'report_digest':version['digest'],'decision_revision':decision['revision'],
            'expected_cells':{self.cell:cell['revision']},'clear_blocks':{self.cell:self.clear_candidates}})
        def read():return self.release.get('/api/v1/qualification-activation',cell=self.cell,id=batch['id'])
        accepted=wait_for(read,lambda v:v['current'] and not v['mixed'] and not v['outcome_unknown'] and v['accepted_hosts']==len(v['hosts']) and len(v['hosts'])>0)
        cell=self.release.get('/api/v1/cell',id=self.cell)
        active=self.release.mutate('qualification-activate','/api/v1/qualification-activation/activate',{
            'batch':batch['id'],'cell':self.cell,'expected':accepted['batch']['revision'],
            'expected_cells':{self.cell:cell['revision']}})
        if active['state'] != 'ACTIVE':raise RuntimeError('P did not record global activation')
        return active
