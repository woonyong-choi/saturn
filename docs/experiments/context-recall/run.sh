#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
script=docs/experiments/context-recall/scripts/01-run.py
case "${1:-}" in
  collect) python3 "$script" prepare; python3 "$script" collect ;;
  process|analyze) python3 "$script" analyze ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh collect|process|analyze|all' >&2; exit 2 ;;
esac
