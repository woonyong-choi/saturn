#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect) python3 scripts/01-collect.py ;;
  label) python3 scripts/02-label.py "$2" ;;
  judge) python3 scripts/03-judge.py ;;
  process) python3 scripts/04-analyze.py ;;
  analyze) python3 scripts/04-analyze.py ;;
  verify) python3 scripts/verify.py ;;
  all) "$0" collect; "$0" label gpt-6-astra; "$0" label gpt-5.6-luna; "$0" label adjudicated; "$0" judge; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh collect|label MODEL|judge|process|analyze|verify|all' >&2; exit 2 ;;
esac
