#!/bin/sh
set -eu
export PYTHONDONTWRITEBYTECODE=1
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
case "${1:-}" in
  collect) python3 "$base/scripts/01-prepare.py"; python3 "$base/scripts/02-collect.py" all ;;
  gold) python3 "$base/scripts/01-prepare.py"; python3 "$base/scripts/02-collect.py" gold ;;
  query) python3 "$base/scripts/02-collect.py" query ;;
  process) python3 "$base/scripts/03-process.py" ;;
  analyze) python3 "$base/scripts/03-process.py"; python3 "$base/scripts/04-analyze.py" ;;
  verify) python3 "$base/scripts/verify.py" ;;
  all) "$0" collect; "$0" analyze; "$0" verify ;;
  *) echo 'usage: run.sh collect|gold|query|process|analyze|verify|all' >&2; exit 2 ;;
esac
