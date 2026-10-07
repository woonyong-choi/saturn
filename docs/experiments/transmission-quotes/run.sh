#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
case "${1:-analyze}" in
  prepare|collect) python3 docs/experiments/transmission-quotes/scripts/01-run.py "$1" ;;
  verify) python3 docs/experiments/transmission-quotes/scripts/01-run.py verify
          python3 docs/experiments/transmission-quotes/scripts/03-archive.py verify ;;
  archive) python3 docs/experiments/transmission-quotes/scripts/03-archive.py archive ;;
  process|analyze) python3 docs/experiments/transmission-quotes/scripts/02-analyze.py ;;
  all) bash "$0" prepare; bash "$0" collect; bash "$0" analyze; bash "$0" archive; bash "$0" verify ;;
  *) exit 2 ;;
esac
