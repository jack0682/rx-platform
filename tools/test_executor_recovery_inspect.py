#!/usr/bin/env python3
"""Inspect the actual stopped E journal without network, state repair or new authority."""
from __future__ import annotations
import json
import re
from pathlib import Path
import test_host_recovery_known as known
from cell_delivery.api import publish_new

FILES = r'''
import hashlib,json,os,stat
from pathlib import Path
root=Path('/data'); records=[]
for path in sorted(root.rglob('*')):
    item=path.lstat(); mode=stat.S_IFMT(item.st_mode)
    assert not stat.S_ISLNK(item.st_mode), 'fixture data symlink'
    assert stat.S_ISREG(item.st_mode) or stat.S_ISDIR(item.st_mode), 'fixture data special file'
    value={'path':str(path.relative_to(root)),'mode':item.st_mode,'mtime_ns':item.st_mtime_ns,'size':item.st_size}
    if stat.S_ISREG(item.st_mode):
        assert item.st_size<=67108864, 'bounded fixture file'
        value['sha256']=hashlib.sha256(path.read_bytes()).hexdigest()
    records.append(value)
    assert len(records)<=4096
print(json.dumps(records,sort_keys=True,separators=(',',':')))
'''

class InspectScenario(known.Scenario):
    def after_executor_stop(self, context: dict, attention: dict) -> dict:
        self.stage='offline-executor-recovery-inspection'
        docker=self.docker
        assert not docker.state(context['e'])['State']['Running']
        def files(label):
            return json.loads(docker.command(context['s_image']['Id'],'/usr/bin/python3',
                ['-c',FILES],[context['e_data']+':/data:ro'],label))
        before_files=files('executor-inspect-files-before')
        before_native=self.native(); before_p=self.oracle('platform'); before_h=self.oracle('host')
        reports=[]
        for index in [1,2]:
            raw=docker.command(context['s_image']['Id'],'/opt/rx/bin/rx-executor-service',
                ['cell','recovery-inspect','/config/executor/cell.json'],context['e_mounts'],
                f'executor-inspect-{index}')
            report=json.loads(raw)
            assert report['schema']=='rx.executor-recovery-inspection.v1'
            for field in ['execution_authorized','network_accessed','session_opened','planner_started',
                          'current_p_state_observed','original_records_modified']:
                assert report[field] is False,field
            assert re.fullmatch(r'[0-9a-f]{64}',report['inspection_digest'])
            assert report['attachment']['run']==attention['status']['run']
            assert report['attachment']['executor_session']==attention['status']['session']
            assert report['run_journal']['stop']['record']==attention['last_run']['stop']
            assert report['unresolved']['pending_stop'] is True
            assert report['unresolved']['attachment_retained'] is True
            assert files(f'executor-inspect-files-after-{index}')==before_files, 'inspection changed source file bytes or metadata'
            reports.append(report)
        assert reports[0]==reports[1], 'the same retained records must produce the same inspection'
        after_p=self.oracle('platform'); after_h=self.oracle('host'); after_native=self.native()
        self.unchanged_authority(before_p,after_p)
        for prefix in ['producer','run','work','part','host-recovery']:
            assert known.rows(before_p,prefix)==known.rows(after_p,prefix),prefix
        assert before_p['outbox']==after_p['outbox']
        assert before_h==after_h, 'offline E inspection affected the Host'
        assert before_native==after_native, 'offline E inspection caused native activity'
        value={'schema':'rx.executor-recovery-inspection-test.v1','status':'PASS','simulation_only':True,
            'platform_image':context['p_image']['Id'],'solutions_image':context['s_image']['Id'],
            'source_file_inventory':before_files,'inspection':reports[0],'repeat_identical':True,
            'network':'Docker network=none for each product inspection command',
            'original_file_bytes_and_mtimes_unchanged':True,'p_and_h_authority_unchanged':True,
            'native_activity_unchanged':True,'executor_original_exit':1,'executor_pending_stop_preserved':True,
            'limits':['Actual product E command on an existing FILE_SIMULATION journal.',
                      'Inspection is local; no fresh P state, peer re-registration, stop resolution or production resume.']}
        publish_new(docker.evidence/'executor-inspection-result.json',value)
        return {'result':'executor-inspection-result.json','inspection_digest':reports[0]['inspection_digest']}

if __name__=='__main__':
    known.main(scenario_type=InspectScenario,extra_validator_sources=(Path(__file__),))
