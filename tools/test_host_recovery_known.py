#!/usr/bin/env python3
"""One withheld FILE_SIMULATION result, P-only restart and approved original lookup.

P/E use shipped binaries; H uses the explicitly supplied test-only Linux fixture
inside the pinned S image. The fixture delegates to the real Host service and
FileDevice. No physical acceptance, operating rebind or automatic resume claim.
Run only in disposable containers. Product processes are the only DB writers.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from cell_delivery.api import Api, Rejected, encoded, publish_new
from cell_delivery.docker import Docker
from cell_delivery.installation import exercise
from cell_delivery.materials import Materials
from cell_delivery.qualification import verified_release_evidence
from cell_delivery.workflow import main_cell

ROOT = Path(__file__).resolve().parents[1]
HOST = 'host/sim'
FIXTURE_BINARY = '/test-artifacts/rx-host-recovery-fixture'
LIMITATIONS = [
    'FILE_SIMULATION only; the independent PHYSICAL cell remains NOT_COMMISSIONED.',
    'H runs an explicit test fixture binary, not the shipped rx-hostd executable. '
    'Its native wrapper withholds a saved FileDevice result; service/RPC/publisher are real.',
    'The marker changes simulated result visibility only. No DB row, capture, authority or evidence is injected.',
    'The signed process requires a postcondition. Native success is recovered, but lost permit/cell continuity preserves overall UNKNOWN/NONE; this test does not authorize a completion decision.',
    'Known original operation lookup only; no Host restart, operating rebind, qualification restoration, '
    'new grant/Arm/permit, native replay, Run/part resume or physical handover acceptance.',
    'The ELF digest and observed source/lock identity are recorded; a source revision alone does not prove build provenance.',
    'The old executor session is revoked. Exit 1 with a retained PENDING stop intent and ATTENTION is verified; clean whole-cell shutdown or executor rebind is not claimed.',
]

# Whitelisted public business records only. No principal/password/session/private-key
# records are selected. A separate network-none process reads the live WAL in one
# read-only transaction. immutable=1 is permitted only after P has exited and no
# uncheckpointed WAL remains; live readers never use it.
ORACLE = r'''
import json,sqlite3,sys
from pathlib import Path
side,mode=sys.argv[1:]
assert side in ('platform','host') and mode in ('live','stopped')
path=Path('/data')/side/(side+'.db')
uri='file:'+str(path)+'?mode=ro'
if mode=='stopped':
    wal=Path(str(path)+'-wal')
    assert not wal.exists() or wal.stat().st_size==0, 'stopped DB has an uncheckpointed WAL'
    uri+='&immutable=1'
groups=(
    {'producer','host','host-binding-baseline','host-link-plan','host-link-fence',
     'cell','run','work','part','permit','mandate','attempt','resource',
     'evidence','evidenceslot','evidencecursor','hostreceipt','fenceack',
     'qualificationbatch','qualificationtask','qualificationcertificate','qualificationhistory',
     'processchange','runtimeinvalidationorigin','host-recovery','host-recovery-fence-proof',
     'host-recovery-receipt-proof'} if side=='platform' else
    {'host','delivery','void','grant','grant-request','resource','cell','permit','request',
     'arm-request','fence-request','accepted-qualification','qualification-request',
     'qualification-history','process-context','configuration-request','configuration-history'})
db=sqlite3.connect(uri,uri=True);db.execute('PRAGMA query_only=ON');db.execute('BEGIN')
rows=[]
for key,revision,raw in db.execute('SELECT key,revision,document FROM entities ORDER BY key'):
    if key.split('/')[0] in groups:
        doc=json.loads(raw)
        rows.append({'key':key,'revision':revision,'schema':doc['schema'],'value':doc['value']})
        assert len(rows)<=4096, 'bounded fixture record count exceeded'
outbox=[];events=[]
if side=='platform':
    for key,state,raw in db.execute('SELECT id,state,document FROM outbox ORDER BY id'):
        value=json.loads(raw)['value']
        if value.get('host')=='host/sim':outbox.append({'id':key,'state':state,'value':value})
    assert len(outbox)<=4096
else:
    for seq,raw in db.execute('SELECT seq,document FROM events ORDER BY seq'):
        doc=json.loads(raw);events.append({'seq':seq,'schema':doc['schema'],'value':doc['value']})
        assert len(events)<=128, 'one-operation fixture must not need a truncated evidence prefix'
db.rollback();db.close()
print(json.dumps({'side':side,'mode':mode,'records':rows,'outbox':outbox,'events':events},sort_keys=True,separators=(',',':')))
'''

NATIVE_FILES = r'''
import hashlib,json,os,stat
from pathlib import Path
def lines(path):
    p=Path(path)
    if not p.exists():return {'records':[],'sha256':hashlib.sha256(b'').hexdigest(),'partial_line':False}
    fd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    with os.fdopen(fd,'rb') as f:
        info=os.fstat(f.fileno());assert stat.S_ISREG(info.st_mode) and info.st_size<=1048576
        raw=f.read(1048577);assert len(raw)<=1048576
    complete=raw.splitlines(keepends=True)
    return {'records':[json.loads(line) for line in complete if line.endswith(b'\n')],
            'sha256':hashlib.sha256(raw).hexdigest(),'partial_line':bool(raw and not raw.endswith(b'\n'))}
p=Path('/data/test-native/result-visible')
marker=None
if p.exists():
    info=p.lstat();marker={'regular':stat.S_ISREG(info.st_mode),'size':info.st_size}
print(json.dumps({'calls':lines('/data/test-native/native-calls.jsonl'),
                  'effects':lines('/data/host/device/effects.jsonl'),'marker':marker},sort_keys=True,separators=(',',':')))
'''


def rows(snapshot: dict, prefix: str) -> list[dict]:
    return [row for row in snapshot['records'] if row['key'].split('/')[0] == prefix]


def single(snapshot: dict, prefix: str, predicate=lambda value: True) -> dict:
    selected = [row for row in rows(snapshot, prefix) if predicate(row['value'])]
    assert len(selected) == 1, (prefix, selected)
    return selected[0]['value']


def wait(check, accepts, label: str, seconds: int = 30):
    deadline = time.monotonic() + seconds
    while True:
        value = check()
        if accepts(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError(f'{label} did not become true; latest observation is retained')
        time.sleep(.2)


class Scenario:
    def __init__(self, docker: Docker, fixture: Path, provenance: dict):
        self.docker, self.fixture, self.provenance = docker, fixture, provenance
        self.c: dict = {}
        self.serial = 0
        self.stage = 'installation'

    def retain(self, label: str, value):
        self.serial += 1
        publish_new(self.docker.evidence / f'{self.serial:04}-{label}.json', value)
        return value

    def oracle(self, side: str, stopped: bool = False) -> dict:
        volume = self.c['p_data' if side == 'platform' else 'h_data']
        self.serial += 1
        value = json.loads(self.docker.command(
            self.c['s_image']['Id'], '/usr/bin/python3',
            ['-c', ORACLE, side, 'stopped' if stopped else 'live'],
            [volume + ':/data:ro'], f'{self.serial:04}-{side}-oracle'))
        return value

    def native(self) -> dict:
        self.serial += 1
        return json.loads(self.docker.command(
            self.c['s_image']['Id'], '/usr/bin/python3', ['-c', NATIVE_FILES],
            [self.c['h_data'] + ':/data:ro'], f'{self.serial:04}-native-files'))

    def capture_native_raw(self, label: str) -> dict:
        destination = self.docker.evidence / label
        destination.mkdir(exist_ok=False)
        inventory = {}
        for source, filename in [('test-native/native-calls.jsonl', 'native-calls.jsonl'),
                                 ('host/device/effects.jsonl', 'effects.jsonl')]:
            target = destination / filename
            try:
                self.docker.extract(self.c['s_image']['Id'], self.c['h_data'], source, target)
                raw = target.read_bytes()
                inventory[filename] = {'sha256': hashlib.sha256(raw).hexdigest(), 'size_bytes': len(raw)}
            except Exception as error:
                inventory[filename] = {'unavailable': type(error).__name__}
        publish_new(destination / 'inventory.json', inventory)
        return inventory

    def host_status(self) -> dict:
        assert self.docker.state(self.c['h'])['State']['Running'], 'the original H must remain alive'
        return self.retain('host-status', json.loads(self.docker.run(
            'exec', self.c['h'], 'cat', '/run/rx-host/host-status.json')))

    def login(self, account: str, label: str) -> Api:
        b = self.c['browser']; final = self.c['final']
        api = Api(b['origin'], final / b['ca'], final / b['certificate'],
                  final / b['private_key'], self.docker.evidence / 'api' / label)
        api.login(account, b['credentials'][account])
        return api

    def direct(self, api: Api, label: str, path: str, body: dict):
        self.retain(label + '-request', {'path': path, 'body': body})
        try:
            return self.retain(label + '-response', api._request(path, encoded(body)))
        except Rejected as error:
            self.retain(label + '-rejected', {'status': error.status, 'body': error.body})
            raise

    def start_services(self, c: dict) -> dict:
        self.c = c
        d = self.docker; image = c['s_image']['Id']; final = c['final']
        hc = d.volume('h-config'); hd = d.volume('h-data'); hr = d.volume('h-runtime')
        d.put(image, hc, final / 'host-config')
        d.prepare_permissions(image, [hc + ':/config', hd + ':/data', hr + ':/work'])
        hm = [hc + ':/config/host:ro', hd + ':/data:rw', hr + ':/run/rx-host:rw']
        d.command(image, '/opt/rx/bin/rx-hostd', ['init', '/config/host/startup.json'], hm, 'h-init')
        # Only the frozen executable is mounted; no source tree, device or signing input.
        hm.append(str(self.fixture.parent) + ':/test-artifacts:ro')
        h = d.start(image, 'h-fixture', 's', FIXTURE_BINARY, ['run', '/config/host/startup.json'], hm)
        ec = d.volume('e-config'); ed = d.volume('e-data'); ew = d.volume('e-work')
        d.put(image, ec, final / 'executor-config')
        d.prepare_permissions(image, [ec + ':/config', ed + ':/data', ew + ':/work'])
        em = [ec + ':/config/executor:ro', ed + ':/data:rw']
        d.command(image, '/opt/rx/bin/rx-executor-service', ['cell', 'init', '/config/executor/cell.json'], em, 'e-init')
        e = d.start(image, 'e', 'e', '/opt/rx/bin/rx-executor-service', ['cell', 'run', '/config/executor/cell.json'], em)
        result = {'h': h, 'e': e, 'h_data': hd, 'e_data': ed,
                  'host_fixture_limitations': LIMITATIONS[1:3]}
        self.c.update(result)
        return result

    def stop(self, name: str, allowed: tuple[int, ...]) -> dict:
        state = self.docker.state(name)['State']
        if state['Running']:
            self.docker.run('kill', '--signal', 'TERM', name)
            state = wait(lambda: self.docker.state(name)['State'], lambda s: not s['Running'], 'normal process stop')
        self.retain('process-stop', {'container': name, 'state': state})
        assert state['ExitCode'] in allowed, state
        return state

    def diagnostics(self) -> None:
        self.retain('diagnostic-stage', {'stage': self.stage})
        for label, read in [('platform', lambda: self.oracle('platform')),
                            ('host', lambda: self.oracle('host')), ('native', self.native),
                            ('host-status', self.host_status)]:
            try:
                self.retain('failure-' + label, read())
            except Exception as error:
                self.retain('failure-' + label, {'unavailable': type(error).__name__})
        for role in ('p', 'h', 'e'):
            if role in self.c:
                try:
                    self.retain('failure-process-' + role, self.docker.state(self.c[role])['State'])
                except Exception as error:
                    self.retain('failure-process-' + role, {'unavailable': type(error).__name__})
        self.capture_native_raw('failure-native-originals')
        self.docker.capture_logs()

    def assert_native(self, observed: dict, operation: str, invocation: str) -> list[dict]:
        assert not observed['calls']['partial_line'] and not observed['effects']['partial_line']
        calls = observed['calls']['records']; effects = observed['effects']['records']
        assert len(effects) == 1 and effects[0]['operation'] == operation and effects[0]['invocation'] == invocation
        assert effects[0]['capture']['captured_at']['clock_id'] == self.c['installation']['clock_id']
        assert sum(v['kind'] == 'SUBMIT' for v in calls) == 1
        assert sum(v['kind'] == 'SUBMIT_RESULT' for v in calls) == 1
        assert all(v['schema'] == 'rx.test-native-call.v1' and v['operation'] == operation
                   and v['invocation'] == invocation and v['observed_at']['clock_id'] == self.c['installation']['clock_id'] for v in calls)
        assert len({v['process_id'] for v in calls}) == 1
        assert all(v['result'] == 'EFFECT_SAVED_RETURN_WITHHELD' for v in calls if v['kind'] == 'SUBMIT_RESULT')
        return calls

    def unchanged_authority(self, before: dict, after: dict) -> None:
        for prefix in ('host', 'host-binding-baseline', 'host-link-plan', 'host-link-fence',
                       'permit', 'mandate', 'attempt', 'resource', 'qualificationbatch',
                       'qualificationtask', 'qualificationcertificate', 'qualificationhistory', 'processchange'):
            assert rows(after, prefix) == rows(before, prefix), f'recovery changed {prefix}'
        cell = self.c['delivery']['cell']
        old = single(before, 'cell', lambda v: v['configuration']['id'] == cell)
        new = single(after, 'cell', lambda v: v['configuration']['id'] == cell)
        for key in ('configuration', 'epoch', 'scope_epochs', 'blocks', 'qualification'):
            assert new[key] == old[key], f'recovery changed cell {key}'
        assert {row['id'] for row in after['outbox']} == {row['id'] for row in before['outbox']}
        for old_message in before['outbox']:
            current = next(row for row in after['outbox'] if row['id'] == old_message['id'])
            assert current['value'] == old_message['value'], 'original outbox body changed'

    def after_commissioning(self, c: dict, active: dict) -> None:
        self.c = c; self.stage = 'one-operation-start'
        d = self.docker; cell = c['delivery']['cell']; operator = c['users']['operator']
        before = operator.get('/api/v1/overview')
        assert main_cell(before, cell)['cell']['value']['commissioning'] == 'COMMISSIONED'
        assert not main_cell(before, cell)['runs']
        config = operator.get('/api/v1/cell', id=cell)
        cfg = config['value']['configuration']
        run = operator.mutate('known-run-create', '/api/v1/runs', {
            'cell': cell, 'recipe_digest': cfg['recipe']['sha256'],
            'site_config_digest': cfg['site_config_digest'], 'expected_cell': config['revision']})
        candidate = wait(lambda: operator.get('/api/v1/run/start-context', cell=cell, run=run['id'], purpose='PRODUCTION', budget_limit='1'),
                         lambda v: v['can_request'], 'quantity-one start context')
        assert candidate['cell'] == cell and candidate['request']['run'] == run['id']
        assert candidate['request']['purpose'] == 'PRODUCTION' and candidate['request']['budget_limit'] == '1'
        attempt = operator.mutate('known-run-start', '/api/v1/runs/start', candidate['request'])
        assert attempt['run'] == run['id'] and attempt['status'] in ('ARMING', 'STARTED')
        observed = wait(self.native, lambda v: len(v['effects']['records']) == 1
                        and any(row['kind'] == 'SUBMIT_RESULT' for row in v['calls']['records'])
                        and not v['calls']['partial_line'], 'withheld native result', 45)
        effect = observed['effects']['records'][0]; operation = effect['operation']; invocation = effect['invocation']
        self.assert_native(observed, operation, invocation)
        assert observed['marker'] is None
        original_h = self.oracle('host')
        host_work = single(original_h, 'delivery', lambda v: v['operation'] == operation)
        assert host_work['state'] == 'SEND_ENTERED' and host_work['invocation'] == invocation
        assert not original_h['events'] and not host_work['evidence_ids']
        old_p = self.oracle('platform')
        old_work = single(old_p, 'work', lambda v: v['operation']['operation_id'] == operation)
        assert old_work['run'] == run['id'] and old_work['operation']['outcome'] == 'NONE'
        started = single(old_p, 'attempt', lambda v: v['id'] == attempt['id'])
        assert started['status'] == 'STARTED' and started['run'] == run['id']
        assert len(rows(old_p, 'work')) == 1 and len(rows(old_p, 'host-binding-baseline')) == 1
        old_producer = single(old_p, 'producer', lambda v: v['principal'] == HOST)
        host_before = self.host_status()
        descriptor = self.retain('host-installation-original', json.loads(d.run('exec', c['h'], 'cat', '/data/host/installation.json')))

        self.stage = 'platform-only-restart'
        original_p = c['p']; first_exit = self.stop(original_p, (0, 2))
        stopped = self.oracle('platform', stopped=True)
        d.capture_logs(); d.run('network', 'disconnect', d.network, original_p)
        d.run('network', 'disconnect', d.front_network, original_p)
        restarted = d.start(c['p_image']['Id'], 'p-restarted', 'p', '/usr/local/bin/rx-platformd',
                            ['run', '/config/startup.json'], c['p_mounts'], [f"127.0.0.1:{c['port']}:8443"])
        c['p'] = restarted
        def relogin():
            assert d.state(restarted)['State']['Running'], 'P exited during restart'
            try:
                return self.login('release', 'restarted-release')
            except (OSError, Rejected):
                return None
        release = wait(relogin, lambda value: value is not None, 'restarted terminal login')
        new_installation = release.get('/api/v1/overview')['installation']
        for field in ('id', 'store_generation', 'clock_id'):
            assert new_installation[field] == c['installation'][field]
        assert new_installation['runtime_boot'] != c['installation']['runtime_boot']
        def reconnected(value):
            producers = rows(value, 'producer')
            return len(producers) == 1 and producers[0]['value']['session'] != old_producer['session'] and cell in producers[0]['value']['cells']
        current = wait(lambda: self.oracle('platform'), reconnected, 'Host publisher reconnect')
        producer = single(current, 'producer')
        for field in ('principal', 'peer_boot', 'journal', 'authentication_binding'):
            assert producer[field] == old_producer[field]
        assert rows(current, 'host') == rows(stopped, 'host')
        assert rows(current, 'host-binding-baseline') == rows(old_p, 'host-binding-baseline')
        cursor = rows(current, 'evidencecursor'); assert len(cursor) == 1
        current = wait(lambda: self.oracle('platform'), lambda value: len(rows(value, 'evidencecursor')) == 1
                       and rows(value, 'evidencecursor')[0]['revision'] >= cursor[0]['revision'] + 2, 'two authenticated empty publication probes')
        assert single(current, 'producer')['session'] == producer['session']
        assert single(current, 'evidencecursor')['through'] == '0'
        assert not single(current, 'evidencecursor')['disputed']
        work = single(current, 'work', lambda v: v['operation']['operation_id'] == operation)
        assert work['operation']['execution_knowledge'] == 'UNKNOWN' and work['operation']['outcome'] == 'NONE'
        assert work['operation']['disposition'] == 'QUARANTINED'
        assert work['permit'] == old_work['permit'] and work['host_journal'] == old_work['host_journal']
        assert work['invocation'] in (None, invocation)
        current_run = single(current, 'run', lambda v: v['id'] == run['id'])
        assert current_run['state'] in ('PAUSED', 'RECOVERY_REQUIRED')
        assert current_run['budget']['limit'] == '1' and len(current_run['budget']['consumptions']) == 1
        assert len(current_run['part_ids']) == 1
        assert not any(row['value']['state'] == 'ISSUED' for row in rows(current, 'permit'))
        assert not any(row['value']['state'] == 'ACTIVE' for row in rows(current, 'mandate'))
        host_after = self.host_status()
        for field in ('host_boot', 'instance', 'installation', 'clock_id'):
            assert host_after[field] == host_before[field]
        assert json.loads(d.run('exec', c['h'], 'cat', '/data/host/installation.json')) == descriptor

        self.stage = 'explicit-recovery-approval'
        context = release.get('/api/v1/host-recovery-context', host=HOST, origin=cell)
        cut = context['context']; assert not cut['blockers'], cut['blockers']
        assert set(cut['operations']) == {operation}, 'approval must name exactly the original operation'
        assert cut['operations'][operation]['permit'] == work['permit']
        assert cut['operations'][operation]['host_journal'] == work['host_journal']
        assert cut['runtime_boot'] == new_installation['runtime_boot']
        assert cut['previous_runtime_boot'] == c['installation']['runtime_boot']
        origins = {row['value']['block']['id']: row['value'] for row in rows(current, 'runtimeinvalidationorigin')}
        assert cut['cells'][cell]['runtime_origins'], 'known RuntimeRestart origin required'
        for block in cut['cells'][cell]['runtime_origins']:
            origin = origins[block]
            assert origin['cell'] == cell and origin['runtime_boot'] == cut['runtime_boot']
            assert origin['previous_runtime_boot'] == cut['previous_runtime_boot']
            assert origin['after']['configuration_digest'] == cut['cells'][cell]['configuration_digest']
        proposal = release.mutate('known-recovery-propose', '/api/v1/host-recoveries', {
            'host': HOST, 'origin': cell, 'expected_context': context['context_digest'], 'expected_cells': context['expected_cells']})
        binding = proposal['view']['binding']; rid = binding['id']
        assert binding['phase'] == 'PROPOSED' and proposal['view']['operation_authorized'] is False
        assert binding['requested_context_digest'] == context['context_digest']
        recovered = release.recover('known-recovery-propose')
        assert recovered['view']['binding']['id'] == rid and recovered['proposal_digest'] == proposal['proposal_digest']
        tasks = binding['fences']; assert set(tasks) == {cell}
        task = tasks[cell]['task']; original_fence = next(row for row in current['outbox'] if row['id'] == task['request'])
        assert task['originating_message'] == original_fence['id'] and original_fence['value']['kind'] == 'FENCE'
        assert original_fence['state'] in ('NEW', 'EMIT_ENTERED')
        after_proposal = self.oracle('platform'); self.unchanged_authority(current, after_proposal)
        assert after_proposal['outbox'] == current['outbox'], 'proposal emitted an outbox action'
        h_recovery_before = self.oracle('host')
        assert rows(h_recovery_before, 'grant') and rows(h_recovery_before, 'arm-request')
        assert len(rows(h_recovery_before, 'accepted-qualification')) == 1
        native_before_approval = self.native(); self.assert_native(native_before_approval, operation, invocation)
        assert native_before_approval['marker'] is None and not h_recovery_before['events']
        approved = release.mutate('known-recovery-approve', '/api/v1/host-recovery/approve', {
            'id': rid, 'expected_revision': binding['revision'], 'proposal_digest': proposal['proposal_digest'], 'expected_cells': context['expected_cells']})
        assert approved['view']['binding']['phase'] == 'RECOVERY_ONLY' and not approved['view']['operation_authorized']
        step = approved['view']['binding']['fences'][cell]
        assert step['task'] == task and step['phase'] == 'ACKNOWLEDGED' and step['payload_digest'] == tasks[cell]['payload_digest']
        assert step['acknowledgment']['invalidation'] == original_fence['id']
        again = release.recover('known-recovery-approve')
        assert again['view']['binding']['fences'] == approved['view']['binding']['fences']
        post_approval = self.oracle('platform'); self.unchanged_authority(current, post_approval)
        assert next(row for row in post_approval['outbox'] if row['id'] == original_fence['id'])['state'] == 'DELIVERED'
        before_wrong = self.native()
        try:
            self.direct(release, 'unapproved-operation', '/api/v1/host-recovery/query', {'id': rid, 'operation': str(uuid.uuid4())})
        except Rejected as error:
            assert error.status in (403, 409, 422)
        else:
            raise AssertionError('query accepted an operation outside the approved set')
        assert self.native() == before_wrong, 'unapproved query entered native lookup'

        def query(label):
            result = self.direct(release, label, '/api/v1/host-recovery/query', {'id': rid, 'operation': operation})
            assert result['binding'] == rid and result['operation'] == operation
            assert result['operation_authorized'] is False and result['evidence_complete'] is False
            assert result['receipt']['operation'] == operation and result['receipt']['invocation'] == invocation
            assert result['receipt']['journal'] == work['host_journal']
            return result
        def lookup_delta(before_native, after_native, expected_result):
            before_calls = self.assert_native(before_native, operation, invocation)
            after_calls = self.assert_native(after_native, operation, invocation)
            assert after_calls[:len(before_calls)] == before_calls
            delta = after_calls[len(before_calls):]
            assert [row['kind'] for row in delta] == ['LOOKUP', 'LOOKUP_RESULT'], delta
            assert delta[-1]['result'] == expected_result, delta

        self.stage = 'hidden-original-query'
        hidden = query('hidden-query'); after_hidden = self.native()
        lookup_delta(before_wrong, after_hidden, 'HIDDEN')
        assert hidden['lookup'] == 'PREFIX_OBSERVED' and hidden['publication_required'] and not hidden['evidence']
        assert not self.oracle('host')['events'] and after_hidden['marker'] is None
        pre_visible_p = self.oracle('platform'); self.unchanged_authority(current, pre_visible_p)
        assert single(pre_visible_p, 'work')['operation']['outcome'] == 'NONE'
        # Observe more authenticated publication probes while only explicit queries
        # may call lookup. The normal dispatcher must not resume this operation.
        cut_cursor = rows(pre_visible_p, 'evidencecursor')[0]['revision']
        wait(lambda: self.oracle('platform'), lambda value: rows(value, 'evidencecursor')[0]['revision'] >= cut_cursor + 2, 'idle probes before visibility')
        assert self.native() == after_hidden, 'an unsolicited lookup occurred after recovery'

        self.stage = 'visible-original-query'
        self.retain('marker-before', {'host_evidence': self.oracle('host')['events'], 'native': after_hidden})
        # The sole test-side state change: an empty mock result-visibility marker.
        # It is outside both DBs and contains no receipt, evidence or authority bytes.
        d.run('exec', c['h'], '/usr/bin/python3', '-c',
              "import os; p='/data/test-native/result-visible'; f=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600); os.fsync(f); os.close(f); d=os.open('/data/test-native',os.O_RDONLY|os.O_DIRECTORY); os.fsync(d); os.close(d)")
        found = query('visible-query'); after_found = self.native()
        lookup_delta(after_hidden, after_found, 'FOUND')
        assert after_found['marker'] == {'regular': True, 'size': 0}
        assert found['lookup'] == 'PREFIX_OBSERVED' and found['publication_required'] and len(found['evidence']) == 1
        evidence = found['evidence'][0]
        assert evidence['operation'] == operation and evidence['invocation'] == invocation
        assert evidence['profile_digest'] == work['intent']['profile_digest']
        for field in ('captured_at', 'device_session', 'status_schema'):
            assert evidence[field] == effect['capture'][field]
        captured_h = self.oracle('host')
        final_host_work = single(captured_h, 'delivery', lambda v: v['operation'] == operation)
        assert final_host_work['state'] == 'RESULT_CAPTURED' and final_host_work['invocation'] == invocation
        assert final_host_work['device_session'] == host_work['device_session']
        assert len(captured_h['events']) == 1
        h_event = captured_h['events'][0]['value']
        assert h_event['evidence_id'] == evidence['id'] and h_event['operation'] == operation and h_event['invocation'] == invocation
        assert h_event['capture'] == effect['capture'], 'lookup must return the original saved native capture'
        assert final_host_work['evidence_ids'] == [evidence['id']]
        published = wait(lambda: self.oracle('platform'), lambda value: any(row['value']['id'] == evidence['id'] for row in rows(value, 'evidence'))
                         and int(single(value, 'evidencecursor')['through']) >= int(captured_h['events'][0]['seq']), 'original publisher full-prefix acknowledgment')
        final_work = single(published, 'work', lambda v: v['operation']['operation_id'] == operation)
        assert single(published, 'evidence') == evidence
        assert single(published, 'evidencecursor')['journal'] == producer['journal']
        assert not single(published, 'evidencecursor')['disputed']
        # This signed process requires native success AND a postcondition. The
        # pre-restart permit cut no longer matches this cell; current true facts
        # cannot retroactively prove completion under the old execution context.
        assert final_work['completion']['kind'] == 'NATIVE' and final_work['completion']['postconditions']
        permit = single(published, 'permit', lambda v: v['id'] == final_work['permit'])
        current_cell = single(published, 'cell', lambda v: v['configuration']['id'] == cell)
        assert current_cell['epoch'] != permit['epoch'] and current_cell['blocks']
        assert final_work['operation']['outcome'] == 'NONE'
        assert final_work['operation']['execution_knowledge'] == 'UNKNOWN'
        assert final_work['operation']['phase'] == 'RECONCILING'
        assert not final_work['operation']['evidence_ids']
        assert evidence['status'] in final_work['completion']['success']
        assert final_work['operation']['disposition'] == 'QUARANTINED' and final_work['invocation'] == invocation
        self.unchanged_authority(current, published)
        repeated = query('captured-prefix-query')
        assert repeated['receipt']['state'] == 'RESULT_CAPTURED'
        assert repeated['lookup'] == 'PREFIX_OBSERVED' and repeated['publication_required']
        assert repeated['evidence'] == [evidence]
        assert self.native() == after_found, 'captured receipt query reentered native lookup or submit'
        assert single(self.oracle('platform'), 'work')['operation']['outcome'] == 'NONE'
        for prefix in ('grant', 'grant-request', 'resource', 'arm-request', 'accepted-qualification',
                       'qualification-request', 'qualification-history', 'process-context',
                       'configuration-request', 'configuration-history'):
            assert rows(captured_h, prefix) == rows(h_recovery_before, prefix), prefix
        final_p = self.oracle('platform'); self.unchanged_authority(current, final_p)
        final_run = single(final_p, 'run', lambda v: v['id'] == run['id'])
        assert final_run['state'] in ('PAUSED', 'RECOVERY_REQUIRED')
        assert final_run['part_ids'] == current_run['part_ids'] and final_run['budget'] == current_run['budget']
        assert rows(final_p, 'part') == rows(current, 'part'), 'lookup resumed or completed a part'
        assert len(rows(final_p, 'work')) == 1
        # ReleaseManager is scoped to the recovered cell. Use the existing
        # operator account with negative-control visibility, after a real relogin.
        negative_operator = self.login('operator', 'post-query-negative-control')
        final_overview = negative_operator.get('/api/v1/overview')
        physical = main_cell(final_overview, c['delivery']['negative_cell'])['cell']['value']
        assert physical['commissioning'] == 'NOT_COMMISSIONED' and physical['qualification'] is None
        restart_blocks = cut['cells'][cell]['runtime_origins']
        assert all(any(block['id'] == bid and block['latched'] for block in single(final_p, 'cell', lambda v: v['configuration']['id'] == cell)['blocks']) for bid in restart_blocks)

        self.stage = 'retained-executor-attention-and-host-shutdown'
        # The old E session is intentionally revoked by P restart. E must retain
        # its unfinished stop intent and report failure, not invent a clean stop.
        executor_exit = self.stop(c['e'], (1,))
        executor_reports = []
        for line in d.run('logs', c['e']).splitlines():
            try:
                report = json.loads(line)
            except ValueError:
                continue
            if isinstance(report, dict) and 'status' in report and 'last_run' in report:
                executor_reports.append(report)
        assert len(executor_reports) == 1, 'one final structured executor report required'
        executor_attention = executor_reports[0]
        status = executor_attention['status']; stop = executor_attention['last_run']
        assert status['phase'] == 'ATTENTION' and status['run'] == run['id']
        assert status['active']['admission'] is False and status['completed_runs'] == '0'
        assert stop['phase'] == 'PENDING' and stop['stop']['phase'] == 'PENDING'
        assert stop['stop']['run'] == run['id'] and stop['stop']['origin_session'] == status['session']
        assert 'UNAUTHENTICATED' in stop['last_error'] and stop['durability_fault'] is None
        self.retain('executor-retained-attention', executor_attention)
        host_exit = self.stop(c['h'], (0,))
        stop_file = c['materials'].temporary / 'known-host-stop.json'
        d.run('cp', c['h'] + ':/run/rx-host/host-status.json', str(stop_file))
        host_stop = json.loads(stop_file.read_text())
        assert host_stop['phase'] in ('STOPPED', 'STOPPED_WITH_RECONCILIATION_REQUIRED')
        assert host_stop['stop']['safe_to_drop'] and not host_stop['stop']['physical_shutdown_assessed']
        last_exit = self.stop(c['p'], (0, 2))
        final_native = self.retain('native-final', self.native())
        self.assert_native(final_native, operation, invocation)
        assert final_native == after_found, 'normal simulation shutdown changed native effects or lookup calls'
        raw_native = self.capture_native_raw('native-originals')
        assert raw_native['native-calls.jsonl']['sha256'] == final_native['calls']['sha256']
        assert raw_native['effects.jsonl']['sha256'] == final_native['effects']['sha256']
        publish_new(d.evidence / 'result.json', {
            'schema': 'rx.host-recovery-known-query-test.v1', 'status': 'PASS', 'simulation_only': True,
            'platform_image': c['p_image']['Id'], 'solutions_image': c['s_image']['Id'], 'fixture': self.provenance,
            'original_installation': c['installation'], 'restarted_installation': new_installation,
            'activation': active['id'], 'binding': rid, 'operation': operation, 'invocation': invocation,
            'run': final_run, 'work': final_work, 'original_fence': original_fence, 'approved': approved,
            'hidden_query': hidden, 'visible_query': found, 'captured_prefix_query': repeated,
            'completion_postcondition_continuity_lost': True, 'operation_outcome_preserved': 'NONE',
            'independent_native_submit_count': 1, 'independent_native_effect_count': 1,
            'native_original_files': raw_native,
            'explicit_query_lookup_results': ['HIDDEN', 'FOUND'], 'host_evidence_count': 1,
            'original_publisher_cursor': single(published, 'evidencecursor'),
            'operating_registration_unchanged': True, 'baseline_unchanged': True, 'restrictions_preserved': True,
            'qualification_reauthorized': False, 'automatic_resume': False, 'physical_control': physical,
            'host_before': host_before, 'host_after_restart': host_after, 'host_stop': host_stop,
            'executor_attention': executor_attention, 'executor_clean_shutdown': False,
            'process_exits': {'p_before': first_exit, 'p_after': last_exit, 'h': host_exit, 'e': executor_exit},
            'oracle': 'Independent network-none, read-only SQLite transactions. Live WAL included; stopped immutable only with empty/absent WAL.',
            'limitations': LIMITATIONS,
        })


def fixture_provenance(source: Path, destination: Path, image: dict) -> dict:
    if source.is_symlink() or not source.is_file():
        raise ValueError('host-fixture must be a regular Linux artifact')
    destination.parent.mkdir()
    shutil.copyfile(source, destination); destination.chmod(0o555)
    raw = destination.read_bytes()
    if len(raw) < 64 or raw[:4] != b'\x7fELF' or raw[4:6] != b'\x02\x01':
        raise ValueError('64-bit little-endian Linux ELF fixture required; a macOS artifact is not supported')
    machine = struct.unpack_from('<H', raw, 18)[0]
    expected = {'amd64': 62, 'arm64': 183}.get(image['Architecture'])
    if expected is None or machine != expected:
        raise ValueError('fixture ELF architecture differs from the selected solutions image')
    solutions = ROOT.parent / 'rx-solutions'
    revision = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=solutions, check=True, capture_output=True, text=True).stdout.strip()
    status = subprocess.run(['git', 'status', '--porcelain'], cwd=solutions, check=True, capture_output=True, text=True).stdout
    return {'binary_sha256': hashlib.sha256(raw).hexdigest(), 'size_bytes': len(raw),
            'elf_machine': machine, 'architecture': image['Architecture'], 'solutions_image': image['Id'],
            'source_revision_observed': revision, 'source_dirty_observed': bool(status),
            'cargo_lock_sha256': hashlib.sha256((solutions / 'Cargo.lock').read_bytes()).hexdigest(),
            'shipped_host_binary': False, 'mount_read_only': True}


def validator_identity() -> str:
    digest = hashlib.sha256(b'RX-KNOWN-RECOVERY-QUERY-ACCEPTANCE-v1\0')
    for path in [Path(__file__), *sorted((ROOT / 'tools/cell_delivery').glob('*.py'))]:
        digest.update(str(path.relative_to(ROOT)).encode() + b'\0'); digest.update(path.read_bytes())
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform-image', default='rx-platform:runtime-draft')
    parser.add_argument('--solutions-image', default='rx-solutions:runtime-draft')
    parser.add_argument('--host-fixture', type=Path, required=True)
    parser.add_argument('--release-evidence', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    args = parser.parse_args()
    args.evidence_dir.mkdir(parents=True, exist_ok=False)
    docker = Docker(args.evidence_dir.resolve()); scenario = None
    try:
        with tempfile.TemporaryDirectory(prefix='rx-known-host-recovery-') as directory:
            temporary = Path(directory)
            p_image = docker.image(args.platform_image); s_image = docker.image(args.solutions_image)
            publish_new(docker.evidence / 'selected-images.json', {
                'platform': {'id': p_image['Id'], 'architecture': p_image['Architecture']},
                'solutions': {'id': s_image['Id'], 'architecture': s_image['Architecture']},
            })
            assert p_image['Architecture'] == s_image['Architecture']
            release_proof = verified_release_evidence(args.release_evidence.resolve(), s_image['Id'])
            fixture = temporary / 'fixture' / 'rx-host-recovery-fixture'
            provenance = fixture_provenance(args.host_fixture, fixture, s_image)
            publish_new(docker.evidence / 'inputs.json', {'platform_image': p_image['Id'], 'solutions_image': s_image['Id'],
                        'fixture': provenance, 'release_evidence': release_proof, 'limitations': LIMITATIONS})
            scenario = Scenario(docker, fixture, provenance)
            try:
                with socket.socket() as connection:
                    connection.bind(('127.0.0.1', 0)); port = connection.getsockname()[1]
                bundle = temporary / 'operator'; holder = docker.holder(s_image['Id'], [])
                docker.run('cp', holder + ':/opt/rx/operator', str(bundle))
                materials = Materials(ROOT, temporary, docker.evidence, docker, s_image['Id'])
                materials.create_seed(s_image['Architecture'])
                package, compiled, compiler = materials.compile(); validator = validator_identity()
                final = materials.finalize(package, compiled, compiler, port, bundle, validator)
                delivery = json.loads((final / 'delivery.json').read_text())
                assert not delivery['qualification_report_generated']
                exercise(docker, materials, final, bundle, p_image, s_image, port, validator,
                         args.release_evidence.resolve(), 'independent',
                         start_services=scenario.start_services, after_commissioning=scenario.after_commissioning)
            except Exception:
                scenario.diagnostics()
                raise
    except Exception as error:
        publish_new(docker.evidence / 'failure.json', {
            'schema': 'rx.host-recovery-known-query-failure.v1', 'status': 'FAIL',
            'stage': scenario.stage if scenario is not None else 'preflight',
            'error_type': type(error).__name__, 'runtime_acceptance': False,
        })
        raise
    finally:
        docker.cleanup()


if __name__ == '__main__':
    main()
