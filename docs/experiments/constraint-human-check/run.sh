#!/bin/sh
set -eu
cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
case "${1:-}" in
  analyze) python3 scripts/01-analyze.py "${2:-}" ;;
  *) echo 'usage: run.sh analyze [원자료 폴더]' >&2; exit 2 ;;
esac
