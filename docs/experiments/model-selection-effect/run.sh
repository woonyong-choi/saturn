#!/bin/sh
set -eu
cd "$(dirname "$0")"
ROOT="$(cd ../../.. && pwd)"

case "${1:-}" in
  selftest)
    python3 scripts/tasks.py "$ROOT/.runtime/st"
    ;;
  collect)
    phase="${2:?dev 또는 confirm}"
    shift 2
    python3 scripts/tasks.py "$ROOT/.runtime/st"
    python3 scripts/01-collect.py "$phase" "$@"
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
    (cd data && shasum -a 256 -c SHA256SUMS)
    python3 scripts/04-verify.py
    ;;
  *)
    echo "사용: ./run.sh selftest|collect dev|collect confirm|process|analyze|verify" >&2
    exit 2
    ;;
esac
