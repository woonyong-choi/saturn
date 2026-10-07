#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-all}" in
  prepare|smoke|collect|verify) python3 scripts/01-run.py "$1" ;;
  process|analyze) python3 scripts/02-analyze.py ;;
  all) python3 scripts/01-run.py collect; python3 scripts/01-run.py verify; python3 scripts/02-analyze.py ;;
  *) exit 2 ;;
esac
