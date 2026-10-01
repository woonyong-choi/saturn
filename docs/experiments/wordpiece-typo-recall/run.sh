#!/bin/sh
# 단어 조각 단위별 오타 재현율 실험. 사용법: ./run.sh collect|process|analyze|verify|all
set -eu
cd "$(dirname "$0")"

case "${1:-}" in
  collect) python3 scripts/01-collect.py collect ;;
  process) python3 scripts/02-process.py ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  verify)
    (cd data && shasum -a 256 -c SHA256SUMS)
    python3 scripts/01-collect.py verify
    ;;
  all)
    "$0" collect
    "$0" verify
    "$0" analyze
    ;;
  *)
    echo "사용법: ./run.sh collect|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
