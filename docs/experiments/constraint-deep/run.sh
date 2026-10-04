#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect)
    python3 "$ROOT/scripts/02-prepare.py"
    python3 "$ROOT/scripts/03-label.py" gpt-6-astra
    python3 "$ROOT/scripts/03-label.py" gpt-5.6-luna
    python3 "$ROOT/scripts/03-label.py" adjudicated
    python3 "$ROOT/scripts/04-judge.py" all
    ;;
  process|analyze) python3 "$ROOT/scripts/05-analyze.py" ;;
  verify) python3 "$ROOT/scripts/verify.py" ;;
  all) "$ROOT/run.sh" collect; "$ROOT/run.sh" verify; "$ROOT/run.sh" analyze ;;
  *) echo 'usage: run.sh {collect|process|analyze|verify|all}' >&2; exit 2 ;;
esac
