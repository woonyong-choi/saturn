#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
case "${1:-analyze}" in
  prepare|collect|verify) python3 docs/experiments/transmission-quotes/scripts/01-run.py "$1" ;;
  process|analyze) python3 docs/experiments/transmission-quotes/scripts/02-analyze.py ;;
  all) "$0" prepare; "$0" collect; "$0" verify; "$0" analyze ;;
  *) exit 2 ;;
esac
