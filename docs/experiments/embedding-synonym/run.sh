#!/bin/sh
# 임베딩 같은 뜻 찾기의 정확도와 비용 실험. 사용법: ./run.sh collect|process|analyze|verify|all
# collect 뒤 data/raw/labels-{실행 id}.csv에 라벨을 붙이고 `python3 scripts/01-collect.py seal`을 실행한다.
set -eu
cd "$(dirname "$0")"

case "${1:-}" in
  collect)
    python3 scripts/01-collect.py collect
    python3 scripts/01-collect.py cost
    ;;
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
    python3 scripts/01-collect.py seal
    "$0" verify
    "$0" analyze
    ;;
  *)
    echo "사용법: ./run.sh collect|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
