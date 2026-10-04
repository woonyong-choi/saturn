#!/usr/bin/env bash
# 사용: ./run.sh collect [주제...] | process | analyze | verify | all
# 수집은 실제 Codex를 호출한다. 호출 상한 120회는 .runtime/codex-calls.jsonl로 누적해 센다.
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-}" in
  collect)
    shift
    python3 scripts/01-collect.py "$@"
    ;;
  process)
    python3 scripts/02-process.py
    ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  verify)
    (cd data && shasum -a 256 -c SHA256SUMS)
    ;;
  all)
    python3 scripts/01-collect.py
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  *)
    echo "usage: $0 collect|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
