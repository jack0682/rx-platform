#!/usr/bin/env python3
"""Run the real P mTLS writer with a separately built, device-free S reporter fixture."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

root = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--solutions', type=Path, required=True)
p.add_argument('--platform-target', type=Path, required=True)
p.add_argument('--solutions-target', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
a = p.parse_args()
solutions = a.solutions.resolve()
evidence = a.evidence.resolve()
targets = [a.platform_target.resolve(), a.solutions_target.resolve()]
for target in targets:
    if target == root or target.is_relative_to(root) or target == solutions or target.is_relative_to(solutions):
        raise SystemExit('Use task-owned target directories outside the product checkouts')
evidence.mkdir(parents=True, exist_ok=False)

def run(repo, label, args, extra=None):
    env = dict(os.environ, **(extra or {}))
    with (evidence / (label + '.log')).open('w') as log:
        result = subprocess.run([str(repo / 'tools/cargo'), *args], cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        raise SystemExit(f'{label} failed; inspect retained log')

run(solutions, 'build-reporter', ['build', '-p', 'rx-supervisor', '--features', 'test-harness', '--bin',
    'rx-resident-report-fixture', '--locked', '--target-dir', str(targets[1])])
executable = targets[1] / 'debug/rx-resident-report-fixture'
for label, test in [
    ('actual-reporting', 'actual_supervisor_child_reports_running_and_owned_exit_through_the_scoped_client'),
    ('actual-reporting-outage', 'actual_reporter_outage_preserves_local_stop_and_owner_approved_restart'),
]:
    run(root, label, ['test', '-p', 'rx-api', '--test', 'resident_reporting',
        test, '--locked', '--target-dir', str(targets[0]), '--', '--ignored', '--exact'],
        {'RX_RESIDENT_REPORT_FIXTURE': str(executable), 'RX_RESIDENT_REPORT_EVIDENCE': str(evidence / 'result.json')})
result = json.loads((evidence / 'result.json').read_text())
outage = json.loads((evidence / 'result.json.outage.json').read_text())
if outage['status'] != 'PASS' or not outage['retained_during_outage'] or outage['local_stop_ms'] >= 2000:
    raise SystemExit('Outage/restart scene did not pass')
if result['status'] != 'PASS':
    raise SystemExit('Reporting scene did not pass')
sources = {}
for label, repo in [('platform', root), ('solutions', solutions)]:
    sources[label] = {'commit': subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip(),
        'working_tree_changes': subprocess.check_output(['git', '-C', str(repo), 'status', '--porcelain'], text=True).splitlines()}
(evidence / 'scope.json').write_text(json.dumps({'sources': sources, 'os': platform.system(), 'architecture': platform.machine(),
    'status': 'PASS_SCOPED_REPORTING', 'physical_execution': 'NOT_PERFORMED',
    'limits': ['Test-owned locally approved software child; not Platform-directed launch',
               'Attributed registry snapshots, not current OS ownership or work permission',
               'Production reporting worker exercised; packaged rx-solutionsd command path is not exercised',
               'Legacy registration migration, process ownership recovery and physical qualification remain open']}, indent=2) + '\n')
print(json.dumps({'status': 'PASS_SCOPED_REPORTING', 'evidence': str(evidence)}))
