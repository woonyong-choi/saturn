#!/bin/sh
# 사용: ./run.sh stage1|stage2|collect|process|analyze|verify|all
#   stage1   1단계 수집, 처리, 분석(진행 조건 판정 포함)
#   stage2   1단계 판정이 `run`일 때만 2단계 수집, 처리, 분석
set -eu
cd "$(dirname "$0")"

verify() {
  if [ ! -f data/SHA256SUMS ]; then
    echo "run.sh: data/SHA256SUMS가 없다" >&2
    exit 1
  fi
  (cd data && shasum -a 256 -c SHA256SUMS)
}

sums() {
  (cd data && find raw -type f -name '*.jsonl' | sort | xargs shasum -a 256 > SHA256SUMS)
}

case "${1:-}" in
  stage1)
    python3 scripts/01-collect.py stage1
    sums
    verify
    python3 scripts/02-process.py stage1
    python3 scripts/03-analyze.py stage1
    ;;
  stage2)
    python3 scripts/01-collect.py stage2
    sums
    verify
    python3 scripts/03-analyze.py stage2
    ;;
  collect)
    python3 scripts/01-collect.py stage1
    sums
    ;;
  process)
    python3 scripts/02-process.py stage1
    ;;
  analyze)
    python3 scripts/02-process.py stage1
    python3 scripts/03-analyze.py stage1
    if ls data/raw/receiver-*.jsonl >/dev/null 2>&1; then
      python3 scripts/03-analyze.py stage2
    fi
    ;;
  verify)
    verify
    ;;
  all)
    "$0" stage1
    if grep -q '"stage2": "run"' results/stage1-gate.json; then
      "$0" stage2
    fi
    ;;
  *)
    echo "사용: ./run.sh stage1|stage2|collect|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
