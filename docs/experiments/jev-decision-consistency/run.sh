#!/bin/sh
# 사용: ./run.sh collect dev|collect confirm|process|analyze|verify
# collect는 환경 변수 SATURN_JUDGE_KEY(Jev API 키)가 필요하다. 키는 같은 셸 명령 안에서만 읽는다.
set -eu
cd "$(dirname "$0")"

case "${1:-}" in
  collect) python3 scripts/01-collect.py "${2:-}" ;;
  process) python3 scripts/02-process.py ;;
  analyze) python3 scripts/02-process.py && python3 scripts/03-analyze.py ;;
  verify) shasum -a 256 -c data/SHA256SUMS && python3 scripts/02-process.py --check && python3 scripts/03-analyze.py --check ;;
  *) echo "사용: ./run.sh collect dev|collect confirm|process|analyze|verify" >&2; exit 2 ;;
esac
