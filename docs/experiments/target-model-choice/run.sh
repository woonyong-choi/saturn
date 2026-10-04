#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect) python3 scripts/01-prepare.py; python3 scripts/02-collect.py "${2:-all}" ;;
  process) python3 scripts/03-analyze.py process ;;
  analyze) python3 scripts/03-analyze.py analyze ;;
  verify) python3 scripts/04-verify.py ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) exit 2 ;;
esac
