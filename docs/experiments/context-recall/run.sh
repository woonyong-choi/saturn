#!/usr/bin/env bash
set -euo pipefail
self="$(cd "$(dirname "$0")" && pwd)/run.sh"
cd "$(dirname "$0")/../../.."
script=docs/experiments/context-recall/scripts/01-run.py
case "${1:-}" in
  collect) python3 "$script" prepare; python3 "$script" collect ;;
  process|analyze) python3 docs/experiments/context-recall/scripts/04-summarize.py ;;
  archive|verify) python3 docs/experiments/context-recall/scripts/05-archive.py "$1" ;;
  all) bash "$self" collect; bash "$self" analyze; bash "$self" archive; bash "$self" verify ;;
  *) echo 'usage: run.sh collect|process|analyze|archive|verify|all' >&2; exit 2 ;;
esac
