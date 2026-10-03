#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PYTHON=${PYTHON:-python3}

case "${1:-}" in
  collect)
    "$PYTHON" "$ROOT/scripts/01-collect.py"
    "$PYTHON" "$ROOT/scripts/02-label.py"
    "$PYTHON" "$ROOT/scripts/03-jev.py"
    "$PYTHON" "$ROOT/scripts/04-process.py"
    "$PYTHON" "$ROOT/scripts/05-analyze.py"
    ;;
  process)
    "$PYTHON" "$ROOT/scripts/04-process.py"
    ;;
  analyze)
    "$PYTHON" "$ROOT/scripts/05-analyze.py"
    ;;
  verify)
    "$PYTHON" "$ROOT/scripts/test_masking.py"
    "$PYTHON" "$ROOT/scripts/01-collect.py" --dry-run
    "$PYTHON" "$ROOT/scripts/03-jev.py" --dry-run
    ;;
  all)
    "$ROOT/run.sh" collect
    ;;
  *)
    echo "사용법: $0 {collect|process|analyze|verify|all}" >&2
    exit 2
    ;;
esac
