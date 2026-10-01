#!/usr/bin/env bash
# 사용: ./run.sh collect|process|analyze|verify|all
# collect에는 SATURN_JUDGE_KEY(Jev 키)가 필요하다.
set -euo pipefail
cd "$(dirname "$0")"

collect() {
  python3 scripts/01-collect.py
  (cd data && find raw -name '*.jsonl' | sort | xargs shasum -a 256 > SHA256SUMS)
}

process() { python3 scripts/02-process.py; }

analyze() {
  process
  python3 scripts/03-analyze.py
}

verify() {
  if [ ! -f data/SHA256SUMS ]; then
    echo "verify: data/SHA256SUMS가 없다. 먼저 collect를 실행한다" >&2
    exit 2
  fi
  (cd data && shasum -a 256 -c SHA256SUMS)
}

case "${1:-}" in
  collect) collect ;;
  process) process ;;
  analyze) analyze ;;
  verify) verify ;;
  all) collect; verify; analyze ;;
  *) echo "사용: $0 collect|process|analyze|verify|all" >&2; exit 2 ;;
esac
