#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect) python3 scripts/run.py prepare; python3 scripts/run.py collect ;;
  process|analyze) python3 scripts/run.py analyze ;;
  verify) python3 scripts/run.py verify ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
