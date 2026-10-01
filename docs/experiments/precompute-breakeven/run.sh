#!/usr/bin/env bash
# Usage: ./run.sh collect|process|analyze|verify|all
set -euo pipefail
cd "$(dirname "$0")"

collect() {
  local run_id
  run_id="$(date -u +%Y%m%dT%H%M%SZ)-$(git rev-parse --short=7 HEAD)"
  python3 scripts/01-collect.py --run-id "$run_id" --out-dir data/raw --env env.json
  (cd data && shasum -a 256 raw/* > SHA256SUMS)
}

verify() {
  (cd data && shasum -a 256 -c --quiet SHA256SUMS)
  python3 scripts/02-process.py --verify
}

process() {
  python3 scripts/02-process.py
}

analyze() {
  process
  python3 scripts/03-analyze.py
}

case "${1:-}" in
  collect) collect ;;
  process) process ;;
  analyze) analyze ;;
  verify) verify ;;
  all) collect; verify; analyze ;;
  *) echo "usage: ./run.sh collect|process|analyze|verify|all" >&2; exit 2 ;;
esac
