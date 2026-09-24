#!/usr/bin/env python3
"""Actual Linux ownership/close/fork passage and unfiltered Host stress for matching source+SDK."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--solutions', required=True, type=Path)
    p.add_argument('--image', required=True)
    p.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    p.add_argument('--evidence', required=True, type=Path)
    p.add_argument('--rounds', type=int, default=50)
    a = p.parse_args()
    if not 1 <= a.rounds <= 200:
        p.error('rounds must be 1..200')
    e = a.evidence.resolve()
    e.mkdir(parents=True, exist_ok=False)
    solutions = a.solutions.resolve()
    commands = []

    def run(label, argv, expect=0):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e / (label + '.stdout')).write_text(r.stdout)
        (e / (label + '.stderr')).write_text(r.stderr)
        commands.append(dict(label=label, argv=argv, exit_code=r.returncode, expected=expect))
        (e / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if expect is not None and r.returncode != expect:
            raise RuntimeError(f'{label}: {r.returncode}, expected {expect}; evidence retained')
        return r.stdout

    run('sdk-check', ['python3', str(ROOT / 'tools/check_host_sdk.py'), str(solutions / 'sdk')])
    image = json.loads(run('runtime', ['docker', 'image', 'inspect', a.image]))[0]['Id']
    builder = json.loads(run('builder', ['docker', 'image', 'inspect', a.builder]))[0]['Id']
    builds = ['docker', 'run', '--rm', '-v', f'{ROOT}:/platform:ro', '-v', f'{solutions}:/solutions:ro',
              '-v', f'{e}:/evidence', '-e', 'CARGO_HOME=/evidence/cargo-home', '-e', 'CARGO_BUILD_JOBS=1']
    def build(label, cwd, args):
        return run(label, builds + ['-w', cwd, '--entrypoint', 'cargo', builder] + args)
    build('fork-probe-build', '/platform/tools/lock_lifetime_probe', ['build', '--locked', '--target-dir', '/evidence/probe-target'])
    output = build('storage-build', '/platform', ['test', '--locked', '-p', 'rx-storage', '--all-features', '--no-run',
                 '--target-dir', '/evidence/platform-target', '--message-format=json'])
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    executables = {v['target']['name']: v['executable'] for v in artifacts if v.get('executable') and v.get('profile', {}).get('test')}
    assert set(executables) == {'rx_storage', 'atomicity', 'ownership'}, executables
    output = build('host-build', '/solutions', ['test', '--locked', '-p', 'rx-host', '--all-features', '--test', 'service', '--no-run',
                 '--target-dir', '/evidence/solutions-target', '--message-format=json'])
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    host = next(v['executable'] for v in artifacts if v.get('executable') and v['target']['name'] == 'service')
    posture = ['docker', 'run', '--rm', '--network', 'none', '--read-only', '--cap-drop', 'ALL',
               '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw', '-v', f'{e}:/evidence:ro']
    probe = '/evidence/probe-target/debug/rx-lock-lifetime-probe'
    result = json.loads(run('fork-overlaps', posture + ['--entrypoint', probe, image]).splitlines()[-1])
    assert result['result'] == 'PASS_NORMAL_RELEASE_AND_LIVE_WRITER_EXCLUSION'
    assert len(result['scenes']) == 11
    for name, executable in executables.items():
        run('storage-' + name, posture + ['--entrypoint', executable, image, '--nocapture'])
    # All rounds are predetermined. Every failure is retained and makes the
    # aggregate fail; a later successful round never erases an earlier one.
    code = """import subprocess,sys,json
rows=[]
for i in range(int(sys.argv[2])):
 r=subprocess.run([sys.argv[1],'--test-threads=16'],capture_output=True,text=True)
 rows.append(dict(round=i,exit_code=r.returncode))
 print('ROUND='+str(i)+' EXIT='+str(r.returncode));print(r.stdout+r.stderr)
print('G1_STRESS='+json.dumps(rows));raise SystemExit(any(v['exit_code'] for v in rows))
"""
    output = run('host-stress', posture + ['--entrypoint', '/usr/bin/python3', image, '-c', code, host, str(a.rounds)], expect=None)
    rounds = json.loads(next(line.split('=', 1)[1] for line in output.splitlines() if line.startswith('G1_STRESS=')))
    assert len(rounds) == a.rounds
    hashes = {}
    for name, executable in {'fork-probe': probe, 'host-service': host, **executables}.items():
        hashes[name] = hashlib.sha256((e / Path(executable).relative_to('/evidence')).read_bytes()).hexdigest()
    result.update(runtime_image=image, builder_image=builder, binary_sha256=hashes, host_stress=rounds,
                  source_platform=str(ROOT), source_solutions=str(solutions),
                  typed_errors='OWNER_ACQUISITION/CONTENTION/FOREIGN_PROCESS/CONNECTION_CLOSE/RELEASE; NO_STRING_ADAPTER',
                  remaining_failures=[r for r in rounds if r['exit_code']],
                  historical_attribution='NO_DEFINITIVE_ATTRIBUTION_OF_F9_SINGLE_FAILURE')
    if result['remaining_failures']:
        result['result'] = 'FAIL_STRESS_REQUIRES_INDIVIDUAL_CONTRAST'
    (e / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'result': result['result'], 'evidence': str(e), 'failed_rounds': result['remaining_failures']}))
    if result['remaining_failures']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
