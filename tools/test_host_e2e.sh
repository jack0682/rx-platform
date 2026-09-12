#!/bin/sh
set -eu
rx_platform=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
rx_solutions=$(dirname -- "$rx_platform")/rx-solutions
(
  cd "$rx_solutions"
  ./tools/cargo build -p rx-host --features test-harness --bin rx-host-sim-server --locked
)
cd "$rx_platform"
RX_HOST_SIM_SERVER="$rx_solutions/target/debug/rx-host-sim-server" ./tools/cargo test -p rx-host-client --test network_e2e --locked -- --ignored
