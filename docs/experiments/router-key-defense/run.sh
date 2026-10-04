#!/bin/sh
set -eu
cd "$(dirname "$0")"

case "${1:-}" in
  collect)
    shift
    python3 scripts/01-collect.py "$@"
    (cd data && find raw -type f -print0 | sort -z | xargs -0 shasum -a 256 > SHA256SUMS)
    ;;
  process)
    python3 scripts/02-process.py
    ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  verify)
    test -f data/SHA256SUMS
    (cd data && shasum -a 256 -c SHA256SUMS)
    ;;
  all)
    "$0" collect keychain-acl codex-sandbox claude-sandbox env hook
    "$0" verify
    "$0" analyze
    ;;
  *)
    echo "사용: ./run.sh collect <keychain-acl|codex-sandbox|claude-sandbox|env|hook>...|process|analyze|verify|all" >&2
    exit 2
    ;;
esac
