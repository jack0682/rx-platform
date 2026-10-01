#!/usr/bin/env python3
"""Exercise P-assigned execution and source investigation in a private signed Linux runtime."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import uuid


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for option in ("solutions","platform-target","solutions-target","registry-cache","evidence"):
        p.add_argument("--"+option,type=Path,required=True)
    p.add_argument("--builder-image",required=True)
    p.add_argument("--runtime-image",required=True)
    a=p.parse_args()
    root=Path(__file__).resolve().parents[1];solutions=a.solutions.resolve();evidence=a.evidence.resolve()
    for target in (a.platform_target.resolve(),a.solutions_target.resolve(),evidence):
        if any(target==r or target.is_relative_to(r) for r in (root,solutions)):
            raise SystemExit("Use task-owned targets/evidence outside the product checkouts")
    evidence.mkdir(parents=True,exist_ok=False)
    out=evidence/"output";out.mkdir()
    results=evidence/"results";results.mkdir();results.chmod(0o777)
    commands=[]
    def run(label,args):
        commands.append({"label":label,"argv":args})
        (evidence/"commands.json").write_text(json.dumps(commands,indent=2)+"\n")
        with (evidence/(label+".log")).open("w") as log:
            result=subprocess.run(args,stdout=log,stderr=subprocess.STDOUT)
        if result.returncode:raise SystemExit(f"{label} failed; retained evidence at {evidence}")
    images={}
    for name,ref in [("builder",a.builder_image),("runtime",a.runtime_image)]:
        v=json.loads(subprocess.check_output(["docker","image","inspect",ref],text=True))[0]
        if v["Os"]!="linux" or v["Architecture"] not in ("arm64","amd64"):
            raise SystemExit("Linux amd64/arm64 images required")
        images[name]={"reference":ref,"id":v["Id"],"architecture":v["Architecture"]}
    if images["builder"]["architecture"]!=images["runtime"]["architecture"]:raise SystemExit("image architectures differ")
    sources={}
    for name,repo in [("platform",root),("solutions",solutions)]:
        dst=evidence/"source"/name;dst.mkdir(parents=True)
        paths=subprocess.check_output(["git","-C",str(repo),"ls-files","--cached","--others","--exclude-standard","-z"]).decode().split("\0")
        inventory={}
        for relative in sorted(set(paths)-{""}):
            path=repo/relative
            if path.is_symlink():raise SystemExit("source symlink requires explicit review: "+relative)
            if not path.is_file():continue
            target=dst/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,target)
            digest=hashlib.sha256(target.read_bytes()).hexdigest()
            if digest!=hashlib.sha256(path.read_bytes()).hexdigest():raise SystemExit("source changed during snapshot")
            inventory[relative]=digest
        sources[name]={"commit":subprocess.check_output(["git","-C",str(repo),"rev-parse","HEAD"],text=True).strip(),"changes":subprocess.check_output(["git","-C",str(repo),"status","--porcelain"],text=True).splitlines(),"files":inventory}
    (evidence/"sources.json").write_text(json.dumps(sources,indent=2)+"\n")
    for target in (a.platform_target.resolve(),a.solutions_target.resolve()):target.mkdir(parents=True,exist_ok=True)
    token=uuid.uuid4().hex[:12]
    build=r'''set -e
cp -a /host-registry /usr/local/cargo/registry
cd /solutions
/usr/local/cargo/bin/cargo build -p rx-supervisor --bin rx-solutionsd --locked --target-dir /targets/solutions > /output/solutions-build.log 2>&1
cp /targets/solutions/debug/rx-solutionsd /output/rx-solutionsd
cd /platform
/usr/local/cargo/bin/cargo test -p rx-api --test resident_execution --no-run --message-format=json --locked --target-dir /targets/platform > /output/platform-build.jsonl 2> /output/platform-build.log
python3 - <<'PY'
import json,pathlib,shutil
messages=[json.loads(x) for x in pathlib.Path('/output/platform-build.jsonl').read_text().splitlines() if x.startswith('{')]
executable=next(x['executable'] for x in messages if x.get('executable') and x.get('target',{}).get('name')=='resident_execution')
shutil.copy2(executable,'/output/resident-execution-test')
PY
'''
    run("build",["docker","run","--rm","--name","rx-execution-build-"+token,"--entrypoint","/bin/bash",
        "-v",str(evidence/"source/platform")+":/platform:ro","-v",str(evidence/"source/solutions")+":/solutions:ro",
        "-v",str(a.platform_target.resolve())+":/targets/platform","-v",str(a.solutions_target.resolve())+":/targets/solutions",
        "-v",str(a.registry_cache.resolve())+":/host-registry:ro","-v",str(out)+":/output",images["builder"]["id"],"-c",build])
    for label,test in [("normal","actual_owner_preparation_grant_child_stop_and_reply_loss"),("outage","actual_p_outage_keeps_local_stop_and_unresolved_delivery"),("cold","actual_cold_manager_loss_preserves_live_original_processes")]:
        run(label,["docker","run","--rm","--name","rx-execution-"+label+"-"+token,"--network","none","--cap-drop=ALL","--security-opt","no-new-privileges","--read-only","--user","10001:10001",
            "--tmpfs","/var/lib/rx-solutions:rw,uid=10001,gid=10001,mode=0700","--tmpfs","/tmp:rw,mode=1777",
            "--entrypoint","/programs/resident-execution-test","-v",str(out)+":/programs:ro","-v",str(results)+":/evidence",
            "-e","RX_RESIDENT_EXECUTION_BIN=/programs/rx-solutionsd","-e",f"RX_RESIDENT_EXECUTION_EVIDENCE=/evidence/{label}.json",
            images["runtime"]["id"],test,"--ignored","--exact","--nocapture"])
        result=json.loads((results/(label+".json")).read_text())
        if label=="cold":
            if result["status"]!="PASS_COLD_SOURCE_INVESTIGATION":raise SystemExit("cold investigation result differs")
        elif result["status"]!="PASS_P_ASSIGNED_ACTUAL_SOFTWARE_EXECUTION" or result["outage"]!=(label=="outage") or result.get("inspection_preserved_source_outbox_and_platform_outcomes") is not True:
            raise SystemExit("scene result differs")
    (evidence/"scope.json").write_text(json.dumps({"images":images,"status":"PASS_SOFTWARE_EXECUTION_AND_SOURCE_INVESTIGATION","scenes":["normal exit inspection","outage exit inspection preserving pending delivery","cold manager loss with original children still present"],"source_snapshots":"sources.json","network":"container loopback only","runtime_user":"10001:10001","read_only_root":True,"physical_execution":"NOT_PERFORMED","limits":["Current source-built daemon; program content comes from the signed runtime image, not a newly released daemon package","P test server uses the actual writer/HTTP/mTLS and actual shared Linux clock, not a full platformd deployment","No work permission, physical completion or independent OS attestation","Source investigation does not release P claims or resume execution; original children in the cold scene are destroyed only with their private container","Active-unknown recovery and full RF01-RF14 remain open"]},indent=2)+"\n")
    print(json.dumps({"status":"PASS_SOFTWARE_EXECUTION_AND_SOURCE_INVESTIGATION","evidence":str(evidence)}))


if __name__=="__main__":main()
