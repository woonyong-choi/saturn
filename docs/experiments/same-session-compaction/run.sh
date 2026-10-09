#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect) python3 scripts/01-collect.py "${2:-48}" ;;
  handoff) python3 scripts/03-handoff.py ;;
  persistent) python3 scripts/05-persistent.py ;;
  analyze|process)
    python3 scripts/02-analyze.py
    python3 scripts/04-handoff-analysis.py
    python3 scripts/06-persistent-analysis.py
    ;;
  verify)
    python3 scripts/02-analyze.py --verify
    python3 scripts/04-handoff-analysis.py --verify
    python3 scripts/06-persistent-analysis.py --verify
    ;;
  all)
    ./run.sh collect "${2:-48}"
    ./run.sh handoff
    ./run.sh persistent
    ./run.sh analyze
    ./run.sh verify
    ;;
  *) echo 'usage: ./run.sh {collect|handoff|persistent|process|analyze|verify|all}' >&2; exit 2 ;;
esac
