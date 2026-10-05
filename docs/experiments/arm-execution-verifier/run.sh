#!/bin/sh
# ARM_VERIFIER_CALLER=stub(기본, 모델 호출 없음) 또는 live(공식 CLI와 TypeSafe HTTPS 호출)
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect) python3 scripts/01-collect.py ;;
  process) python3 scripts/02-process.py ;;
  analyze) python3 scripts/03-analyze.py ;;
  verify) python3 scripts/04-verify.py ;;
  all) python3 scripts/01-collect.py; python3 scripts/02-process.py; python3 scripts/03-analyze.py; python3 scripts/04-verify.py ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
