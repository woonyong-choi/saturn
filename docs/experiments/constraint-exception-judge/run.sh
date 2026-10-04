#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
 collect) python3 scripts/01-prepare.py; python3 scripts/02-collect.py; python3 scripts/03-grade.py ;;
 process) python3 scripts/04-analyze.py process ;;
 analyze) python3 scripts/04-analyze.py process; python3 scripts/04-analyze.py analyze ;;
 verify) python3 scripts/verify.py ;;
 all) "$0" collect; "$0" analyze; "$0" verify ;;
 *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
