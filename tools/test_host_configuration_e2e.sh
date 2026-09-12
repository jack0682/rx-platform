#!/bin/sh
set -eu
platform_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
solutions_dir=$(dirname -- "$platform_dir")/rx-solutions
[ "$#" = 1 ] || { echo 'usage: test_host_configuration_e2e.sh NEW_ABSOLUTE_EVIDENCE_DIRECTORY' >&2; exit 2; }
case "$1" in /*) ;; *) exit 2;; esac
[ ! -e "$1-coordinator" ] || { echo 'coordinator evidence directory exists' >&2; exit 2; }
[ ! -e "$1" ] || { echo 'evidence directory exists' >&2; exit 2; }
(cd "$solutions_dir" && ./tools/cargo build -p rx-host --features test-harness --bin rx-host-sim-server --locked)
cd "$platform_dir"
RX_HOST_SIM_SERVER="$solutions_dir/target/debug/rx-host-sim-server" RX_HOST_CONFIGURATION_EVIDENCE="$1" ./tools/cargo test -p rx-host-client --test configuration_e2e --locked -- --ignored --nocapture
