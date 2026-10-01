#!/bin/sh
# 사용: ./run.sh collect|process|analyze|verify|all
# collect는 환경 변수 SATURN_JUDGE_KEY(Jev API 키)가 필요하다.
set -eu
cd "$(dirname "$0")"

collect() { python3 scripts/01-collect.py; }
process() { python3 scripts/02-process.py; }
analyze() { process && python3 scripts/03-analyze.py; }
verify() { shasum -a 256 -c data/SHA256SUMS; }

case "${1:-}" in
  collect) collect ;;
  process) process ;;
  analyze) analyze ;;
  verify) verify ;;
  all) collect && verify && analyze ;;
  *) echo "사용: ./run.sh collect|process|analyze|verify|all" >&2; exit 2 ;;
esac
