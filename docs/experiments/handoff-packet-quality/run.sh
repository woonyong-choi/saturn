#!/bin/sh
# 사용: ./run.sh collect [--limit N|--resume 실행 id]|process|analyze|verify|all
set -eu
cd "$(dirname "$0")"

verify() {
  if [ ! -f data/SHA256SUMS ]; then
    echo "run.sh: data/SHA256SUMS가 없다" >&2
    exit 1
  fi
  (cd data && shasum -a 256 -c SHA256SUMS)
}

command="${1:-}"
[ "$#" -gt 0 ] && shift

case "$command" in
  collect)
    python3 scripts/01-collect.py "$@"
    (cd data && find raw -type f -name '*.jsonl' | sort | xargs shasum -a 256 > SHA256SUMS)
    ;;
  process)
    python3 scripts/02-process.py
    ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    python3 scripts/04-diagnose.py
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
    echo "사용: ./run.sh collect [--limit N|--resume 실행 id]|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
