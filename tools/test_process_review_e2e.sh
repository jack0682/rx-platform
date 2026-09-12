#!/usr/bin/env bash
set -euo pipefail
platform_dir="$(cd "$(dirname "$0")/.." && pwd)"
solutions_dir="$(cd "$platform_dir/../rx-solutions" && pwd)"
if [ "$#" -ne 1 ] || [[ "$1" != /* ]]; then
  echo 'usage: test_process_review_e2e.sh NEW_ABSOLUTE_EVIDENCE_DIRECTORY' >&2
  exit 2
fi
if [ -e "$1" ]; then echo 'evidence directory must be new' >&2; exit 2; fi
(cd "$solutions_dir" && ./tools/cargo build -p rx-process-package --bin rx-process-package --locked)
cd "$platform_dir"
RX_PROCESS_PACKAGE_BIN="$solutions_dir/target/debug/rx-process-package" RX_REVIEW_EVIDENCE_DIR="$1" \
  ./tools/cargo test -p rx-api --test http process_review_real_compiler_report_crosses_http_and_approval_rechecks_store --locked -- --ignored --exact --nocapture
