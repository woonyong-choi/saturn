#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  inventory|seal|collect|process|analyze|verify) python3 scripts/01-collect.py "$1" ;;
  all) python3 scripts/01-collect.py collect; python3 scripts/01-collect.py process; python3 scripts/01-collect.py analyze; python3 scripts/01-collect.py verify ;;
  *) echo 'usage: run.sh inventory|seal|collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
