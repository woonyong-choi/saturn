#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect)
    python3 "$ROOT/scripts/01-prepare.py"
    python3 "$ROOT/scripts/02-label.py" gpt-6-astra
    python3 "$ROOT/scripts/02-label.py" gpt-5.6-luna
    python3 "$ROOT/scripts/02-label.py" adjudicated
    python3 "$ROOT/scripts/03-collect.py" original
    python3 "$ROOT/scripts/03-collect.py" extension
    ;;
  process) python3 "$ROOT/scripts/04-process.py" ;;
  analyze) "$ROOT/run.sh" process; python3 "$ROOT/scripts/05-analyze.py" ;;
  verify) python3 "$ROOT/scripts/verify.py" ;;
  all) "$ROOT/run.sh" collect; "$ROOT/run.sh" analyze; "$ROOT/run.sh" verify ;;
  *) echo 'usage: run.sh {collect|process|analyze|verify|all}' >&2; exit 2 ;;
esac
