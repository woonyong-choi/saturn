#!/bin/sh
set -eu
export PYTHONDONTWRITEBYTECODE=1
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
case "${1:-}" in
  collect) python3 "$base/scripts/01-prepare.py"; python3 "$base/scripts/02-collect.py" all; python3 "$base/scripts/seal.py" ;;
  gold) python3 "$base/scripts/01-prepare.py"; python3 "$base/scripts/02-collect.py" gold ;;
  query) python3 "$base/scripts/02-collect.py" query; python3 "$base/scripts/seal.py" ;;
  seal) python3 "$base/scripts/seal.py" ;;
  process) python3 "$base/scripts/03-process.py" ;;
  analyze) python3 "$base/scripts/03-process.py"; python3 "$base/scripts/04-analyze.py"; python3 "$base/scripts/report.py" ;;
  verify) python3 "$base/scripts/verify.py" ;;
  figures) python3 "$base/scripts/figures.py" ;;
  all) "$0" collect; "$0" analyze; "$0" figures; "$0" verify ;;
  *) echo 'usage: run.sh collect|gold|query|seal|process|analyze|figures|verify|all' >&2; exit 2 ;;
esac
