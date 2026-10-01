#!/bin/sh
# 사용: ./run.sh collect|process|analyze|verify|all
set -eu
cd "$(dirname "$0")"

verify() {
  if [ ! -f data/SHA256SUMS ]; then
    echo "run.sh: data/SHA256SUMS가 없다" >&2
    exit 1
  fi
  (cd data && shasum -a 256 -c SHA256SUMS)
}

case "${1:-}" in
  collect)
    python3 scripts/01-collect.py
    (cd data && find raw -type f -name '*.jsonl' | sort | xargs shasum -a 256 > SHA256SUMS)
    ;;
  process)
    python3 scripts/02-process.py
    ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  verify)
    verify
    ;;
  all)
    "$0" collect
    verify
    "$0" analyze
    ;;
  *)
    echo "사용: ./run.sh collect|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
