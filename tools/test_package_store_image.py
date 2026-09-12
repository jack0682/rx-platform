#!/usr/bin/env python3
"""Offline package-store acceptance, on disposable data only; never drives hardware."""
import argparse, hashlib, json, shutil, subprocess, tempfile, uuid
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--image', default='rx-platform:runtime-draft')
parser.add_argument('--package', type=Path, required=True)
parser.add_argument('--policy', type=Path, required=True)
parser.add_argument('--evidence', type=Path, required=True)
args = parser.parse_args()

def run(*argv, success=True):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=90)
    if success and result.returncode != 0:
        raise AssertionError(result.stderr or result.stdout)
    if not success:
        assert result.returncode != 0, result.stdout
    return result

def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, separators=(',', ':'))+'\n')

policy = json.loads(args.policy.read_bytes())
assert not policy['assets'] and not policy['dependencies'], 'This image smoke uses a self-contained test package.'
volume = 'rx-package-store-test-'+uuid.uuid4().hex[:12]
checks = []
with tempfile.TemporaryDirectory(prefix='rx-package-store-image-') as temporary:
    fixture = Path(temporary)
    source = fixture/'incoming'/'package'
    shutil.copytree(args.package, source)
    shutil.copyfile(args.policy, fixture/'policy.json')
    source_hashes = {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source.rglob('*') if p.is_file()}
    config = {'schema':'rx.package-store-config.v1','store_root':'/data/packages','import_root':'/fixture/incoming','policy_file':'/fixture/policy.json','policy_digest':hashlib.sha256((fixture/'policy.json').read_bytes()).hexdigest()}
    write_json(fixture/'config.json', config)
    bad_config = dict(config, policy_digest='0'*64)
    write_json(fixture/'bad-pin.json', bad_config)
    revoked_policy = dict(policy, keys=[])
    write_json(fixture/'revoked-policy.json', revoked_policy)
    write_json(fixture/'revoked-config.json', dict(config, policy_file='/fixture/revoked-policy.json', policy_digest=hashlib.sha256((fixture/'revoked-policy.json').read_bytes()).hexdigest()))
    try:
        run('docker','volume','create',volume)
        run('docker','run','--rm','--network','none','--user','0','-v',volume+':/data','--entrypoint','/bin/sh',args.image,'-c','chmod 700 /data && chown 10001:10001 /data')
        common = ['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','-v',volume+':/data:rw','-v',str(fixture)+':/fixture:ro']
        def command(*argv, success=True):
            return run(*common,'--entrypoint','/usr/local/bin/rx-package-store',args.image,*argv,success=success)
        def shell(script,*argv):
            return run(*common,'--entrypoint','/bin/sh',args.image,'-c',script,'check',*argv)
        command('import','/fixture/bad-pin.json','package',success=False)
        command('import','/fixture/revoked-config.json','package',success=False)
        command('import','/fixture/config.json','../package',success=False)
        shell('test ! -e /data/packages')
        checks.append('invalid pin, revoked publisher and traversal rejected before store creation')
        first = json.loads(command('import','/fixture/config.json','package').stdout)
        assert first['status']=='CONTENT_VERIFIED_NOT_ADMITTED'
        assert first['activation_authorized'] is False and first['content_semantics_verified'] is False
        again = json.loads(command('import','/fixture/config.json','package').stdout)
        assert again['object']==first['object']
        checks.append('same signed content imported by independent processes has one object identity')
        object_id=first['object']
        identity = [object_id['manifest'], object_id['signature']]
        original_source = (source/'process'/'source.json').read_bytes()
        (source/'process'/'source.json').write_bytes(b'changed source after verified storage')
        command('import','/fixture/config.json','package',success=False)
        verified = json.loads(command('verify','/fixture/config.json',*identity).stdout)
        assert verified['object']==object_id
        checks.append('source mutation rejected on import; stored independent bytes verify after restart')
        command('verify','/fixture/revoked-config.json',*identity,success=False)
        command('verify','/fixture/bad-pin.json',*identity,success=False)
        checks.append('every verification loads current pinned policy; historic success cannot bypass revocation')
        (source/'process'/'source.json').write_bytes(original_source)
        stored_path='/data/packages/'+object_id['manifest']+'-'+object_id['signature']+'/process/source.json'
        shell('printf corrupted > "$1"',stored_path)
        command('verify','/fixture/config.json',*identity,success=False)
        command('import','/fixture/config.json','package',success=False)
        assert shell('cat "$1"',stored_path).stdout=='corrupted'
        checks.append('stored corruption rejected and not repaired by repeated import')
        assert set(shell('ls -A /data').stdout.splitlines())=={'packages'}
        checks.append('no authority database, run, process launch or activation artifact created')
        image=json.loads(run('docker','image','inspect',args.image).stdout)[0]
        assert image['Config']['User']=='10001:10001'
        result={'schema':'rx.package-store-image-smoke.v1','status':'PASS','image_id':image['Id'],'os':image['Os'],'architecture':image['Architecture'],'user':image['Config']['User'],'read_only_root':True,'network':'none','cap_drop':['ALL'],'package_files_sha256':source_hashes,'policy_sha256':config['policy_digest'],'import':first,'verify':verified,'checks':checks,'limitations':['test-only policy; no production trust registration','offline content storage only; no user/cell admission, semantic approval or activation','no forced power-loss or failing-disk durability test','data volume removed after test']}
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'status':'PASS','image':image['Id'],'checks':len(checks)}))
    finally:
        subprocess.run(['docker','volume','rm',volume],capture_output=True)
