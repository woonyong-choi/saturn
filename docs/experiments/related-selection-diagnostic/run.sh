#!/bin/sh
set -eu
cd "$(dirname "$0")"

case "${1:-}" in
  collect)
    python3 scripts/01-collect.py
    ;;
  process)
    python3 scripts/02-process.py "${2:?실행 id 필요}"
    ;;
  analyze)
    python3 scripts/02-process.py "${2:?실행 id 필요}"
    python3 scripts/03-analyze.py
    ;;
  verify)
    run_id="${2:?실행 id 필요}"
    python3 scripts/02-process.py "$run_id"
    (cd "../../.."/.runtime/related-selection-diagnostic/"$run_id" && shasum -a 256 -c "../../../docs/experiments/related-selection-diagnostic/data/SHA256SUMS")
    python3 scripts/03-analyze.py
    cp results/summary.json "../../.."/.runtime/related-selection-diagnostic/"$run_id"/verify-summary.json
    python3 scripts/03-analyze.py
    cmp results/summary.json "../../.."/.runtime/related-selection-diagnostic/"$run_id"/verify-summary.json
    ;;
  all)
    SATURN_RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$(git rev-parse --short=7 HEAD)"
    export SATURN_RUN_ID
    "$0" collect
    "$0" analyze "$SATURN_RUN_ID"
    "$0" verify "$SATURN_RUN_ID"
    ;;
  *)
    echo "사용: ./run.sh collect|process|analyze|verify|all [실행 id]" >&2
    exit 2
    ;;
esac
