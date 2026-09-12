#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
solutions="$root/../rx-solutions"
(cd "$solutions" && ./tools/cargo build -p rx-executor --features test-harness --bin rx-executor-worker-fixture --bin rx-executor-decision-fixture --bin rx-executor-service-fixture --locked)
image=$(docker image inspect rx-solutions:executor-validation --format '{{.Id}}')
if [ "$#" -gt 0 ]; then
  if [ -e "$1" ]; then printf '%s\n' 'Output directory must not already exist' >&2; exit 1; fi
  mkdir -p "$1"
  RX_EXECUTOR_WORKER_OUTPUT=$(CDPATH= cd -- "$1" && pwd)
  export RX_EXECUTOR_WORKER_OUTPUT
fi
RX_EXECUTOR_SERVICE_FIXTURE="$solutions/target/debug/rx-executor-service-fixture" RX_EXECUTOR_DECISION_FIXTURE="$solutions/target/debug/rx-executor-decision-fixture" RX_EXECUTOR_WORKER_FIXTURE="$solutions/target/debug/rx-executor-worker-fixture" RX_BT_REQUEST_IMAGE="$image" "$root/tools/cargo" test --manifest-path "$root/Cargo.toml" -p rx-api --test executor_ingress --locked durable_s_worker -- --ignored
