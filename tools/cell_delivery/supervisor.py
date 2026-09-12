"""Fresh P + supervised S acceptance composition; no admission or journal fabrication."""
from __future__ import annotations

import hashlib
import json
import shutil
import time
import uuid
from pathlib import Path

from .api import encoded, publish_new


CONFIGURATION = '/config/supervisor/startup.json'
STATE = '/var/lib/rx-solutions/cell-delivery'
OPTIONS = {'poll_ms': 50, 'communication_grace_ms': 5000, 'stop_timeout_ms': 10000}
START_TIMEOUT = 80
STOP_TIMEOUT = 75


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _load(path: Path) -> dict | list:
    _require(path.is_file() and not path.is_symlink(), f'regular fixture input required: {path.name}')
    def unique(pairs):
        value = {}
        for key, item in pairs:
            _require(key not in value, 'duplicate JSON key')
            value[key] = item
        return value
    return json.loads(path.read_bytes(), object_pairs_hook=unique)


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _domain_digest(domain: str, value: object) -> str:
    # The exported fixture subset has the same bytes under encoded() and JCS.
    # Do not silently extend this into a general canonical JSON implementation.
    def check(item):
        if item is None or type(item) is bool:
            return
        if type(item) is str:
            _require(item.isascii(), 'fixture digest accepts ASCII strings only')
        elif type(item) is int:
            _require(abs(item) <= 9_007_199_254_740_991, 'fixture digest integer exceeds exact range')
        elif type(item) is list:
            for child in item:
                check(child)
        elif type(item) is dict:
            for key, child in item.items():
                _require(type(key) is str, 'JSON key must be a string')
                check(key)
                check(child)
        else:
            raise ValueError('fixture digest rejects floats and unsupported values')
    check(value)
    return hashlib.sha256(domain.encode('ascii') + b'\n' + encoded(value)).hexdigest()


def compose(c: dict) -> dict:
    """Project already-patched deployment files into named, release-owned recipes."""
    final = Path(c['final'])
    installation = c['installation']
    host = _load(final / 'host-config/startup.json')
    executor = _load(final / 'executor-config/cell.json')
    bindings = _load(final / 'host-config/bindings.json')
    _require(host['schema'] == 'rx.host-startup.v1', 'Host startup schema differs')
    _require(host['backend'] == {'kind': 'FILE_SIMULATION'}, 'only FILE_SIMULATION is supported')
    _require(host['data_directory'] == '/data/host' and host['runtime_directory'] == '/run/rx-host',
             'Host fixture roots differ')
    _require(host['bind'] == '0.0.0.0:7444', 'Host fixture listener differs')
    _require(host['bindings']['path'] == '/config/host/bindings.json', 'Host binding path differs')
    _require(host['bindings']['sha256'] == _sha(final / 'host-config/bindings.json'), 'Host binding pin differs')
    _require(executor['schema'] == 'rx.executor-cell-service.v1', 'Executor startup schema differs')
    _require(executor['service_root'] == '/data/executor' and executor.get('options') == OPTIONS,
             'Executor must use the exact normalized fixture root and explicit options')
    expected = executor['expected_service']
    scope = expected['scope']
    _require(str(uuid.UUID(expected['journal'])) == expected['journal'], 'Executor journal must be a canonical UUID')
    _require(host['installation'] == scope['installation'] == installation['id'], 'installation identity differs')
    _require(host['release_digest'] == scope['release'], 'H/E release identity differs')
    _require(host['publisher']['store_generation'] == scope['store_generation'] == installation['store_generation'],
             'actual P store generation must be patched before composition')
    _require(host['publisher']['uri'] == executor['platform']['uri'] == 'https://p:7443', 'P endpoint differs')
    _require(host['publisher']['server_name'] == executor['platform']['server_name'] == 'p', 'P TLS name differs')
    _require(executor['engine']['path'] == '/opt/rx/bin/rx-bt-engine', 'release engine path differs')
    _require(len(bindings) == 1, 'this acceptance fixture requires exactly one managed SIMULATION cell')
    binding = bindings[0]
    _require(binding['cell'] == scope['cell'] and binding['host'] == host['host'], 'H/E cell provider differs')
    _require(binding['definition']['sha256'] == scope['definition'], 'H/E cell definition differs')
    _require(binding['environment'] == 'SIMULATION' and binding['platform'] == installation['id'], 'Host binding scope differs')
    _require(set(host['allowed_platform_certificates'].values()) == {installation['id']}, 'P Hello peer differs')

    host_scope = {'role': 'HOST', 'installation': host['installation'], 'host': host['host'],
                  'installation_identity': _domain_digest('RX-HOST-INSTALLATION-CONFIG-v1',
                      [host['installation'], host['host'], host['backend'], host['bindings']['sha256']])}
    # /data/executor is a real directory at this exact path in S, never macOS realpath.
    executor_scope = {'role': 'EXECUTOR', 'installation': scope['installation'], 'cell': scope['cell'],
                      'service_journal': expected['journal'],
                      'configuration_digest': _domain_digest('RX-EXECUTOR-CELL-CONFIG-v1', executor)}
    services = {
        'host': {'configuration': {'path': '/config/host/startup.json', 'sha256': _sha(final / 'host-config/startup.json')}, 'scope': host_scope},
        'executor': {'configuration': {'path': '/config/executor/cell.json', 'sha256': _sha(final / 'executor-config/cell.json')}, 'scope': executor_scope},
    }
    def process(name, program, dependencies, parameters, startup, shutdown):
        return {'id': name, 'program': program, 'parameters': parameters, 'depends_on': dependencies,
                'startup_timeout_ms': str(startup), 'shutdown_timeout_ms': str(shutdown),
                'restart_limit': '0', 'restart_backoff_ms': '500'}
    output = final / 'supervisor-config'
    output.mkdir(exist_ok=True)
    path = output / 'startup.json'
    previous = _load(path) if path.exists() else None
    configuration = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'cell-delivery',
                     'plan': {'schema': 'rx.solutions-process-plan.v1',
                              'id': previous['plan']['id'] if previous else str(uuid.uuid4()),
                              'environment': 'SIMULATION', 'profiles': [], 'processes': [
                                  process('status', 'rx/status-http', [], {'bind': '127.0.0.1', 'port': '8081'}, 10000, 5000),
                                  process('host', 'rx/service/host', ['status'], {}, 30000, 30000),
                                  process('executor', 'rx/service/executor', ['host'], {}, 30000, 30000)]},
                     'services': services}
    publish_new(path, configuration)
    pins = {'schema': 'rx.delivery-supervisor-pins.v1', 'configuration_sha256': _sha(path),
            'services': services, 'expected_executor_service': expected,
            'binding_sha256': host['bindings']['sha256'], 'engine': executor['engine'],
            'authority_created': False, 'simulation_only': True}
    publish_new(final / 'supervisor-pins.json', pins)
    return {'configuration': configuration, 'pins': pins, 'configuration_source': str(output)}


def _role_sources(c: dict) -> dict[str, Path]:
    """Copy only the exporter's declared runtime files, preserving TLS key modes."""
    final = Path(c['final'])
    browser = _load(final / 'browser-fixture.json')
    destination = c['materials'].temporary / 'supervised-role-configs'
    destination.mkdir(exist_ok=False)
    sources = {}
    for role, label, expected_target in [('H', 'host', '/config/host'), ('E', 'executor', '/config/executor')]:
        descriptor = browser['allowed_mounts'][role]
        _require(descriptor['target'] == expected_target, 'role mount target differs')
        expected_source = 'host-config' if role == 'H' else 'executor-config'
        _require(descriptor['source'] == expected_source, 'role mount source differs')
        source = final / expected_source
        actual_keys = {str(p.relative_to(source)) for p in source.rglob('*.key')}
        _require(actual_keys == set(descriptor['private_keys']), 'unexpected role private key inventory')
        _require(not list(source.rglob('signing-fixtures.json')), 'private signing fixture must remain outside S')
        target = destination / label
        target.mkdir()
        for relative in descriptor['files']:
            rel = Path(relative)
            _require(not rel.is_absolute() and '..' not in rel.parts and relative != 'signing-fixtures.json', 'invalid role file path')
            file = source
            _require(not source.is_symlink(), 'role root must not be a symlink')
            for part in rel.parts:
                file = file / part
                _require(not file.is_symlink(), 'role input must not contain symlink aliases')
            _require(file.is_file(), 'role input must be a regular file')
            copy = target / rel
            copy.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, copy)
        sources[label] = target
    return sources


def _prepare_roots(c: dict, mounts: list[str]) -> None:
    # Fixed owned volume roots only; no shell interpolation, host directories or devices.
    script = '''import os
roots = ['/config/host', '/config/executor', '/config/supervisor', '/data', '/var/lib/rx-solutions', '/run/rx-solutions', '/run/rx-host']
assert not os.path.lexists('/data/host'), 'Host init target must be absent'
assert not os.path.lexists('/var/lib/rx-solutions/cell-delivery'), 'supervisor state must be fresh'
os.makedirs('/data/executor', exist_ok=True)
assert not os.listdir('/data/executor'), 'Executor init root must be unused'
for root in roots:
    assert os.path.isdir(root) and not os.path.islink(root), 'real mount root required'
    for directory, children, files in os.walk(root):
        assert not os.path.islink(directory)
        os.chown(directory, 10001, 10001)
        os.chmod(directory, 0o700)
        for filename in files:
            path = os.path.join(directory, filename)
            assert not os.path.islink(path)
            os.chown(path, 10001, 10001)
            if filename.endswith('.key'): os.chmod(path, 0o600)
assert os.path.realpath('/data/executor') == '/data/executor'
'''
    args = ['run', '--rm', '--network', 'none', '--user', '0', '--read-only', '--cap-drop', 'ALL',
            '--cap-add', 'CHOWN', '--cap-add', 'FOWNER', '--cap-add', 'DAC_OVERRIDE',
            '--security-opt', 'no-new-privileges', '--entrypoint', '/usr/bin/python3']
    for mount in mounts:
        args += ['-v', mount.replace(':ro', ':rw')]
    c['docker'].run(*args, c['s_image']['Id'], '-c', script)


def _reports(text: str) -> list[dict]:
    reports = []
    for line in text.splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get('schema') == 'rx.supervisor-status.v1':
            reports.append(value)
    return reports


def _ready_order(reports: list[dict], plan: str, digest: str) -> None:
    started = set()
    for report in reports:
        _require(report['state']['plan'] == plan and report['state']['plan_digest'] == digest, 'supervisor plan identity changed')
        _require(not report['control_prepared'] and not report['physical_shutdown_assessed'], 'supervision claimed control authority')
        records = report['state']['records']
        _require(set(records) == {'status', 'host', 'executor'}, 'selected process inventory differs')
        for child, parent in [('host', 'status'), ('executor', 'host')]:
            if child not in started and records[child]['phase'] in ('STARTING', 'PROCESS_READY'):
                _require(records[parent]['phase'] == 'PROCESS_READY', f'{child} started before {parent} readiness')
                started.add(child)
    _require(started == {'host', 'executor'}, 'managed readiness order was not observed')


def _copy(c: dict, volume: str, source: str, destination: Path) -> None:
    c['docker'].extract(c['s_image']['Id'], volume, source, destination)


def capture(c: dict) -> dict:
    """Best-effort public status/owned logs capture, including partial-start failures."""
    meta = c.get('supervisor')
    if not meta:
        return {'available': False, 'errors': []}
    meta['captures'] = meta.get('captures', 0) + 1
    destination = c['docker'].evidence / 'supervisor' / f"capture-{meta['captures']:03}"
    try:
        destination.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        return {'available': False, 'errors': [f'capture directory: {error}']}
    errors = []
    reports = []
    if c.get('s'):
        try:
            c['docker'].capture_logs()
            text = c['docker'].run('logs', c['s'])
            (destination / 'supervisor.stdout.log').write_text(text + '\n')
            reports = _reports(text)
            publish_new(destination / 'container-state.json', c['docker'].state(c['s'])['State'])
        except Exception as error:
            errors.append(f'supervisor output: {error}')
    for volume, source, name in [
        (meta['volumes'].get('state'), 'cell-delivery/init-logs', 'init-logs'),
        (meta['volumes'].get('state'), 'cell-delivery/logs', 'child-logs'),
        (meta['volumes'].get('runtime'), '', 'guarded-status'),
    ]:
        if volume is None:
            continue
        try:
            _copy(c, volume, source, destination / name)
        except Exception as error:
            errors.append(f'{name}: {error}')
    result = {'available': True, 'directory': str(destination), 'errors': errors,
              'reports': reports, 'last_report': reports[-1] if reports else None}
    try:
        publish_new(destination / 'capture.json', result)
    except OSError as error:
        errors.append(f'capture manifest: {error}')
    return result


def _descriptor(c: dict, volume: str, source: str, name: str) -> dict:
    path = c['materials'].temporary / name
    _copy(c, volume, source, path)
    return _load(path)


def start(c: dict) -> dict:
    """Initialize fixed metadata offline, then start one real S supervisor container."""
    local = dict(c)
    docker = c['docker']
    image = c['s_image']['Id']
    composed = compose(c)
    sources = _role_sources(c)
    volumes = {}
    meta = dict(composed, volumes=volumes)
    local['supervisor'] = meta
    try:
        for name in ['host-config', 'executor-config', 'configuration', 'data', 'state', 'runtime', 'host-runtime']:
            volumes[name] = docker.volume('supervised-' + name)
        docker.put(image, volumes['host-config'], sources['host'])
        docker.put(image, volumes['executor-config'], sources['executor'])
        docker.put(image, volumes['configuration'], Path(composed['configuration_source']))
        mounts = [volumes['host-config'] + ':/config/host:ro', volumes['executor-config'] + ':/config/executor:ro',
                  volumes['configuration'] + ':/config/supervisor:ro', volumes['data'] + ':/data:rw',
                  volumes['state'] + ':/var/lib/rx-solutions:rw', volumes['runtime'] + ':/run/rx-solutions:rw',
                  volumes['host-runtime'] + ':/run/rx-host:rw']
        meta['mounts'] = mounts
        _prepare_roots(local, mounts)
        host_inspection = json.loads(docker.command(image, '/opt/rx/bin/rx-hostd',
            ['inspect', '/config/host/startup.json'], mounts, 'supervisor-host-inspect'))
        host_scope = composed['pins']['services']['host']['scope']
        _require(host_inspection['identity'] == host_scope['installation_identity']
                 and host_inspection['installation'] == host_scope['installation']
                 and host_inspection['host'] == host_scope['host'], 'actual Host inspection identity differs')
        _require(host_inspection['activation_authorized'] is False and host_inspection['native_processes_started'] == 0,
                 'offline inspection unexpectedly authorized or started a native process')
        inspection = json.loads(docker.command(image, '/opt/rx/bin/rx-solutionsd',
            ['inspect', CONFIGURATION], mounts, 'supervisor-inspect'))
        _require(inspection['schema'] == 'rx.solutions-plan-inspection.v1'
                 and inspection['protocol_guarded_services'] is True and inspection['control_prepared'] is False
                 and inspection['physical_qualification'] == 'NOT_PERFORMED', 'supervisor inspection differs')
        meta['inspection'] = inspection
        meta['host_inspection'] = host_inspection
        initialized = json.loads(docker.command(image, '/opt/rx/bin/rx-solutionsd',
            ['init', CONFIGURATION], mounts, 'supervisor-init'))
        _require([(step['program'], step['role'], step['phase'], step['exit_code']) for step in initialized['steps']]
                 == [('rx/service/host', 'HOST', 'COMPLETED', 0), ('rx/service/executor', 'EXECUTOR', 'COMPLETED', 0)],
                 'actual named initialization did not complete in dependency order')
        meta['initialized'] = initialized
        host = _descriptor(local, volumes['data'], 'host/installation.json', 'supervised-host-installation.json')
        executor = _descriptor(local, volumes['data'], 'executor/cell-installation.json', 'supervised-executor-installation.json')
        _require(host['identity'] == host_scope['installation_identity'] and host['installation'] == host_scope['installation']
                 and host['host'] == host_scope['host'], 'initialized Host descriptor differs')
        executor_scope = composed['pins']['services']['executor']['scope']
        _require(executor['schema'] == 'rx.executor-cell-installation.v1'
                 and executor['configuration_digest'] == executor_scope['configuration_digest']
                 and executor['service_root'] == '/data/executor'
                 and executor['expected_service'] == composed['pins']['expected_executor_service'], 'initialized Executor descriptor differs')
        meta['descriptors'] = {'host': host, 'executor': executor}
        public = {'schema': 'rx.delivery-supervisor-initialized.v1', 'pins': composed['pins'],
                  'inspection': inspection, 'host_inspection': host_inspection, 'initialized': initialized,
                  'descriptors': meta['descriptors'], 'authority_created': False}
        publish_new(docker.evidence / 'supervisor-initialized.json', public)
        c['materials'].preserve_public(Path(composed['configuration_source']) / 'startup.json', 'supervisor-configuration')
        c['materials'].preserve_public(Path(c['final']) / 'supervisor-pins.json', 'supervisor-pins')
        # Docker.start may fail after create; retain its known owned name for failure capture.
        local['s'] = docker.prefix + '-s'
        meta['container'] = local['s']
        container = docker.start(image, 's', 's', '/opt/rx/bin/rx-solutionsd', ['run', CONFIGURATION], mounts)
        local.update(s=container, h=container, e=container, h_data=volumes['data'], e_data=volumes['data'])
        deadline = time.monotonic() + START_TIMEOUT
        while True:
            _require(docker.state(container)['State']['Running'], 'S supervisor exited before readiness')
            reports = _reports(docker.run('logs', container))
            if reports:
                report = reports[-1]
                _require(not report['state']['stop_requested'], 'supervisor restricted the composition during startup')
                if all(row['phase'] == 'PROCESS_READY' for row in report['state']['records'].values()):
                    _ready_order(reports, composed['configuration']['plan']['id'], inspection['plan_digest'])
                    break
            _require(time.monotonic() < deadline, 'supervised H/E readiness deadline exceeded')
            time.sleep(.1)
        meta['ready_report'] = report
        ready_status = {}
        ready_directory = c['materials'].temporary / 'supervisor-ready-status'
        ready_directory.mkdir()
        for role in ['host', 'executor']:
            record = report['state']['records'][role]
            instance = record['instance']
            _require(str(uuid.UUID(instance)) == instance, 'managed instance must be a canonical UUID')
            path = ready_directory / f'{role}.{instance}.json'
            _copy(local, volumes['runtime'], path.name, path)
            value = _load(path)
            _require(value['schema'] == 'rx.protocol-guarded-status.v1'
                     and value['scope'] == composed['pins']['services'][role]['scope']
                     and value['instance'] == instance and value['pid'] == record['pid']
                     and value['state'] == {'kind': 'READY'} and int(value['sequence']) > 0
                     and value['observed_at']['clock_id'] == c['installation']['clock_id'],
                     f'{role} actual READY status differs from the supervised owner')
            ready_status[role] = value
        meta['ready_status'] = ready_status
        publish_new(docker.evidence / 'supervisor-ready.json', {'reports': reports, 'readiness_order_verified': True})
        c['materials'].preserve_public(ready_directory, 'supervisor-ready-status')
        return {key: local[key] for key in ['s', 'h', 'e', 'h_data', 'e_data', 'supervisor']}
    except Exception:
        try:
            capture(local)
        except Exception:
            pass  # Preserve the original failure; Docker.cleanup still retains container logs.
        raise


def _terminal(c: dict, report: dict, captured: dict) -> dict:
    meta = c['supervisor']
    records = report['state']['records']
    proofs = {}
    for role in ['host', 'executor']:
        record = records[role]
        exit_proof = record['guarded_exit']
        _require(record['phase'] == 'EXITED' and record['exit_code'] == 0
                 and exit_proof is not None and exit_proof['state'] == 'CONFIRMED', f'{role} stop is unconfirmed')
        _require(str(uuid.UUID(record['instance'])) == record['instance'], 'terminal instance must be a canonical UUID')
        path = Path(captured['directory']) / 'guarded-status' / f"{role}.{record['instance']}.json"
        value = _load(path)
        _require(_sha(path) == exit_proof['payload_digest'], f'{role} terminal payload digest differs')
        _require(value['schema'] == 'rx.protocol-guarded-status.v1'
                 and value['scope'] == meta['pins']['services'][role]['scope']
                 and value['instance'] == record['instance'] and value['pid'] == record['pid']
                 and value['sequence'] == exit_proof['sequence'] and value['observed_at'] == exit_proof['observed_at'],
                 f'{role} terminal status owner/scope/cut differs')
        _require(value['state'] == {'kind': 'STOPPED', 'reconciliation_required': exit_proof['reconciliation_required']},
                 f'{role} did not publish its correlated terminal state')
        _require(value['observed_at']['clock_id'] == c['installation']['clock_id'], f'{role} terminal clock differs from P')
        proofs[role] = {'record': record, 'status': value, 'payload_sha256': _sha(path)}
    return proofs


def stop(c: dict) -> dict:
    """Send one TERM to PID 1; observe confirmed child exits without forcing or stopping P."""
    docker = c['docker']
    meta = c['supervisor']
    _require(not meta.get('term_sent', False), 'supervisor stop is already requested; capture existing outcome')
    _require(docker.state(c['s'])['State']['Running'], 'supervisor exited before the requested normal stop')
    # Finishing an inner RunService must leave the resident cell service alive.
    executor_record = meta['ready_report']['state']['records']['executor']
    log_path = STATE + '/logs/' + executor_record['instance'] + '.stdout.log'
    deadline = time.monotonic() + 15
    idle_since = None
    while True:
        _require(docker.state(c['s'])['State']['Running'], 'supervisor exited during completed-run retirement')
        output = docker.run('exec', c['s'], 'cat', log_path)
        states = [json.loads(line) for line in output.splitlines() if line.strip()]
        current = next((state for state in reversed(states) if 'completed_runs' in state), None)
        idle = current is not None and current['phase'] == 'IDLE' and current['completed_runs'] == '1'
        idle = idle and current['run'] is None and current['attachment'] is None
        if idle:
            idle_since = time.monotonic() if idle_since is None else idle_since
            if time.monotonic() - idle_since >= 1:
                break
        else:
            idle_since = None
        _require(time.monotonic() < deadline, 'resident executor did not return to stable idle after one completed run')
        time.sleep(.1)
    publish_new(docker.evidence / 'supervisor-completed-idle.json', {
        'schema':'rx.delivery-resident-idle.v1', 'status':current, 'observed_idle_seconds_at_least':1,
        'supervisor_alive':True, 'same_executor_instance':executor_record['instance'],
    })
    # Record entry before invoking Docker: an uncertain response is never permission to resend.
    meta['term_sent'] = True
    try:
        docker.run('kill', '--signal', 'TERM', c['s'])
        deadline = time.monotonic() + STOP_TIMEOUT
        while docker.state(c['s'])['State']['Running']:
            _require(time.monotonic() < deadline, 'supervisor stop remains unconfirmed; no forced termination')
            time.sleep(.1)
        captured = capture(c)
        _require(not captured['errors'], 'supervisor terminal evidence capture failed')
        reports = captured['reports']
        _require(bool(reports), 'supervisor emitted no status report')
        _ready_order(reports, meta['configuration']['plan']['id'], meta['inspection']['plan_digest'])
        for report in reports:
            records = report['state']['records']
            if records['host']['phase'] == 'STOP_REQUESTED':
                _require(records['executor']['phase'] == 'EXITED', 'Host stopped before its Executor exited')
            if records['status']['phase'] == 'STOP_REQUESTED':
                _require(records['host']['phase'] == 'EXITED', 'diagnostics stopped before Host exited')
        report = reports[-1]
        _require(report['state']['stop_requested'] and report['all_exited'] and report['guarded_shutdown_confirmed'],
                 'supervisor has no confirmed managed shutdown')
        _require(docker.state(c['s'])['State']['ExitCode'] == 0, 'supervisor exited unsuccessfully')
        _require(all(row['phase'] == 'EXITED' and row['exit_code'] == 0 for row in report['state']['records'].values()),
                 'selected process did not complete normal exit')
        proofs = _terminal(c, report, captured)
        result = {'schema': 'rx.delivery-supervisor-stop.v1', 'supervisor_report': report,
                  'terminal_proofs': proofs, 'evidence': captured['directory'], 'term_requests': 1,
                  'forced_termination': False, 'physical_shutdown_assessed': False}
        publish_new(docker.evidence / 'supervisor-stop.json', result)
        return result
    except Exception:
        try:
            capture(c)
        except Exception:
            pass
        raise
