#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
 collect) shift; python3 scripts/collect.py "$@" ;;
 process|analyze) python3 scripts/analyze.py ;;
 verify) python3 scripts/analyze.py --verify ;;
 all) python3 scripts/collect.py; python3 scripts/analyze.py; python3 scripts/analyze.py --verify ;;
 *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
