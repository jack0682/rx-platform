#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
solutions="$root/../rx-solutions"
(cd "$solutions" && ./tools/cargo build -p rx-executor --features test-harness --bin rx-executor-read-fixture --locked)
if [ "$#" -gt 0 ]; then
  if [ -e "$1" ]; then printf '%s\n' 'Output directory must not already exist' >&2; exit 1; fi
  export RX_EXECUTOR_FRAME_OUTPUT="$1"
fi
RX_EXECUTOR_READ_FIXTURE="$solutions/target/debug/rx-executor-read-fixture" "$root/tools/cargo" test --manifest-path "$root/Cargo.toml" -p rx-api --test executor_ingress --locked solutions_reader_restores -- --ignored
