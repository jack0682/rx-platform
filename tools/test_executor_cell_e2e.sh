#!/usr/bin/env bash
set -euo pipefail
if [ "$#" -ne 1 ]; then
  printf '%s\n' 'Usage: tools/test_executor_cell_e2e.sh NEW_OUTPUT_DIRECTORY' >&2
  exit 1
fi
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
solutions="$root/../rx-solutions"
if [ -e "$1" ] || [ -L "$1" ]; then
  printf '%s\n' 'Output directory must not already exist' >&2
  exit 1
fi
mkdir -p -- "$(dirname -- "$1")"
mkdir -- "$1"
RX_EXECUTOR_CELL_OUTPUT=$(CDPATH= cd -- "$1" && pwd)
export RX_EXECUTOR_CELL_OUTPUT
# Inspect the existing test-only C++ fixture image. Never build, pull or select a mutable tag in S.
image=$(docker image inspect "${RX_BT_REQUEST_IMAGE:-rx-solutions:executor-validation}" --format '{{.Id}}')
if [[ ! "$image" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  printf '%s\n' 'docker inspect did not return one immutable image ID' >&2
  exit 1
fi
docker image inspect "$image" > "$RX_EXECUTOR_CELL_OUTPUT/cpp-image-inspect.json"
printf '%s\n' "$image" > "$RX_EXECUTOR_CELL_OUTPUT/cpp-image-id.txt"
(
  cd "$solutions"
  ./tools/cargo build -p rx-executor --features test-harness --bin rx-executor-cell-service-fixture --locked
) 2>&1 | tee "$RX_EXECUTOR_CELL_OUTPUT/build-s-fixture.log"
export RX_EXECUTOR_CELL_SERVICE_FIXTURE="$solutions/target/debug/rx-executor-cell-service-fixture"
export RX_BT_REQUEST_IMAGE="$image"
"$root/tools/cargo" test --manifest-path "$root/Cargo.toml" -p rx-api --test executor_ingress --locked \
  resident_executor_discovers_two_operator_started_runs_in_one_process_and_session \
  -- --ignored --exact --nocapture 2>&1 | tee "$RX_EXECUTOR_CELL_OUTPUT/test-p-resident.log"
# A stale test filter must not be mistaken for a successful integration run.
test -f "$RX_EXECUTOR_CELL_OUTPUT/resident-cell/p-evidence.json"
test -f "$RX_EXECUTOR_CELL_OUTPUT/resident-cell/service-report.json"
