#!/bin/sh
# Registered loopback P + actual S worker; Host evidence remains synthetic.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$#" -ne 2 ]; then printf '%s\n' 'usage: test_executor_v2_registered.sh SOLUTIONS_CHECKOUT NEW_OUTPUT_DIRECTORY' >&2; exit 2; fi
solutions=$(CDPATH= cd -- "$1" && pwd)
output=$2
if [ -e "$output" ]; then printf '%s\n' 'Output directory already exists' >&2; exit 2; fi
python3 "$root/tools/check_host_sdk.py" "$solutions/sdk"
cargo_command=${RX_V2_CARGO:-"$root/tools/cargo"}
p_target=${RX_PLATFORM_V2_TARGET:-"$root/target"}
s_target=${RX_EXECUTOR_V2_TARGET:-"$solutions/target"}
"$cargo_command" build --manifest-path "$solutions/Cargo.toml" --target-dir "$s_target" --locked -p rx-executor --features test-harness --bin rx-executor-v2-registered-fixture
"$cargo_command" build --manifest-path "$root/Cargo.toml" --target-dir "$p_target" --locked -p rx-api --example execution_v2_registered
mkdir "$output"
output=$(CDPATH= cd -- "$output" && pwd)
# Each read has a real 100ms deadline. Do not run concurrent builds or load cases here.
for mode in normal begin submit complete; do
  "$p_target/debug/examples/execution_v2_registered" "$s_target/debug/rx-executor-v2-registered-fixture" "$output/$mode" "$mode" > "$output/$mode.log" 2>&1
done
python3 - "$output" <<'PY'
import json,sys
from pathlib import Path
root=Path(sys.argv[1])
runs={m:json.loads((root/m/'result.json').read_text()) for m in ['normal','begin','submit','complete']}
assert all(r['status']=='REGISTERED_P_EXECUTOR_V2_PASS' for r in runs.values())
(root/'summary.json').write_text(json.dumps({'status':'REGISTERED_EXECUTOR_V2_PRECHECK_PASS','runs':runs,'native_host_effects':'NOT_TESTED','frozen_v1_injection':'NOT_TESTED','human_M3':'NOT_ACCEPTED'},indent=2)+'\n')
print('Registered P/Executor2: normal + three committed-reply-loss controls PASS')
PY
