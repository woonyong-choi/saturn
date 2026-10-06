#!/bin/sh
# ARM_VERIFIER_CALLER=stub(기본, 모델 호출 없음) 또는 live(공식 CLI와 TypeSafe HTTPS 호출)
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect) python3 scripts/01-collect.py ;;
  process) python3 scripts/02-process.py ;;
  analyze) python3 scripts/03-analyze.py ;;
  verify) python3 scripts/04-verify.py ;;
  online-collect) python3 scripts/05-online.py collect ;;
  online-process) python3 scripts/05-online.py process ;;
  online-verify) python3 scripts/05-online.py verify ;;
  preserve-plan) python3 scripts/06-preserve.py plan ;;
  preserve-seal) python3 scripts/06-preserve.py seal ;;
  preserve-collect) python3 scripts/06-preserve.py collect ;;
  preserve-process) python3 scripts/06-preserve.py process ;;
  preserve-verify) python3 scripts/06-preserve.py verify ;;
  all) python3 scripts/01-collect.py; python3 scripts/02-process.py; python3 scripts/03-analyze.py; python3 scripts/04-verify.py ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all|online-collect|online-process|online-verify|preserve-plan|preserve-seal|preserve-collect|preserve-process|preserve-verify' >&2; exit 2 ;;
esac
