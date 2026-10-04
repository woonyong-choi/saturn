#!/bin/sh
set -eu
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  audit) python3 "$base/scripts/01-audit.py" ;;
  prepare) python3 "$base/scripts/02-prepare.py" ;;
  collect) python3 "$base/scripts/02-prepare.py"; python3 "$base/scripts/03-collect.py" "${2:-all}" ;;
  process) python3 "$base/scripts/04-process.py" ;;
  analyze) python3 "$base/scripts/04-process.py"; python3 "$base/scripts/05-analyze.py" ;;
  verify) python3 "$base/scripts/06-verify.py" ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh audit|prepare|collect [gold|query|all]|process|analyze|verify|all' >&2; exit 2 ;;
esac
