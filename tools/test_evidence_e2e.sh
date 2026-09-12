#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
solutions="$root/../rx-solutions"
(cd "$solutions" && ./tools/cargo build -p rx-host --features test-harness --bin rx-host-sim-server --locked)
RX_HOST_SIM_SERVER="$solutions/target/debug/rx-host-sim-server" "$root/tools/cargo" test --manifest-path "$root/Cargo.toml" -p rx-api --test evidence_ingress --locked -- --ignored
