#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
EXP="$ROOT/docs/experiments/continuation-newtask"
export PYTHONDONTWRITEBYTECODE=1
export TMPDIR="$ROOT/.local/experiments/continuation-newtask/runtime"
export XDG_CACHE_HOME="$TMPDIR/cache"
mkdir -p "$TMPDIR"
case "${1:-}" in
  collect)
    shift
    if [ "${1:-}" = prepare ]; then
      python3 "$EXP/scripts/01-collect.py"
    else
      python3 "$EXP/scripts/02-run.py" "$@"
    fi
    ;;
  process) python3 "$EXP/scripts/03-analyze.py" ;;
  analyze) python3 "$EXP/scripts/03-analyze.py"; python3 "$EXP/scripts/report.py" ;;
  verify) python3 "$EXP/scripts/verify.py" ;;
  all)
    "$0" collect prepare
    "$0" collect gpt-6-astra
    "$0" collect gpt-5.6-luna
    "$0" collect adjudicated
    "$0" collect judge
    "$0" analyze
    "$0" verify
    ;;
  *) exit 2 ;;
esac
