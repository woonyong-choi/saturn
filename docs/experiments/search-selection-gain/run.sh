#!/bin/sh
# dev: 개발 계열의 Jev 응답만 수집. lock: tau 고정. collect: 확인 평가 수집(설계와 고정값이 커밋된 뒤). 나머지는 원응답에서 가공·검증
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  dev) python3 scripts/01-collect.py dev ;;
  lock) python3 scripts/lock.py ;;
  collect) python3 scripts/01-collect.py sealed ;;
  process) python3 scripts/02-process.py ;;
  analyze) python3 scripts/03-analyze.py ;;
  verify) python3 scripts/04-verify.py ;;
  *) echo 'usage: run.sh dev|lock|collect|process|analyze|verify' >&2; exit 2 ;;
esac
