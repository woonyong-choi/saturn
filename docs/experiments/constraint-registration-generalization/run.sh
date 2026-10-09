#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  prepare) python3 -B scripts/01-prepare.py ;;
  collect) python3 -B scripts/02-collect.py ;;
  process) python3 -B scripts/03-analyze.py process ;;
  analyze)
    python3 -B scripts/03-analyze.py
    python3 -B rounding_audit.py
    ;;
  verify) python3 -B scripts/03-analyze.py verify ;;
  all)
    python3 -B scripts/02-collect.py
    python3 -B scripts/03-analyze.py verify
    python3 -B scripts/03-analyze.py
    python3 -B rounding_audit.py
    ;;
  *) echo 'usage: sh run.sh prepare|collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
