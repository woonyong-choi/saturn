#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  collect) python3 scripts/01-collect.py prepare; python3 scripts/01-collect.py label gpt-6-astra; python3 scripts/01-collect.py label gpt-5.6-luna; python3 scripts/01-collect.py label adjudicated; python3 scripts/01-collect.py judge ;;
  prepare) python3 scripts/01-collect.py prepare ;;
  label) python3 scripts/01-collect.py label "$2" ;;
  judge) python3 scripts/01-collect.py judge ;;
  process|analyze)
    if [ ! -f ../../../.local/experiments/continuation-misjoin/extraction-audit.json ]; then
      python3 scripts/audit-extraction.py
    fi
    python3 scripts/02-analyze.py
    python3 scripts/report.py
    ;;
  verify) python3 scripts/verify.py ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
