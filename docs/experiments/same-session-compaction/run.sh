#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect) python3 scripts/01-collect.py "${2:-48}" ;;
  analyze|process) python3 scripts/02-analyze.py ;;
  verify) python3 scripts/02-analyze.py --verify ;;
  all) python3 scripts/01-collect.py "${2:-48}" && python3 scripts/02-analyze.py --verify ;;
  *) echo 'usage: ./run.sh {collect|process|analyze|verify|all}' >&2; exit 2 ;;
esac
