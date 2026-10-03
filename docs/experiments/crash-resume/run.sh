#!/bin/sh
set -eu

case "${1:-}" in
  collect)
    python3 scripts/01-collect.py
    find data/raw -maxdepth 1 -type f -name '*.jsonl' -print0 | sort -z | xargs -0 shasum -a 256 > data/SHA256SUMS
    ;;
  process)
    python3 scripts/02-process.py
    ;;
  analyze)
    python3 scripts/03-analyze.py
    ;;
  verify)
    test -f design.md
    test -f data/README.md
    test -f data/SHA256SUMS
    test -f ../README.md
    test -d scripts
    python3 scripts/02-process.py --check
    python3 scripts/03-analyze.py --check
    first_raw=$(git log --reverse --format=%H -- data/raw | head -n 1)
    design_commit=$(git log -n 1 --format=%H -- design.md)
    test -n "$first_raw"
    git merge-base --is-ancestor "$design_commit" "$first_raw"
    ;;
  all)
    "$0" collect
    "$0" process
    "$0" analyze
    "$0" verify
    ;;
  *)
    echo "사용법: $0 {collect|process|analyze|verify|all}" >&2
    exit 2
    ;;
esac
