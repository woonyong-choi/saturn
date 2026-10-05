#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect) python3 scripts/01-collect.py ;;
  process|analyze) python3 scripts/02-analyze.py ;;
  verify) python3 scripts/03-verify.py ;;
  all) python3 scripts/01-collect.py; python3 scripts/02-analyze.py; python3 scripts/03-verify.py ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
