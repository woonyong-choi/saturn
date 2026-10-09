#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
case "${1:-analyze}" in
  analyze|process) python3 docs/experiments/recall-selection/scripts/04-summarize.py ;;
  verify) python3 docs/experiments/recall-selection/scripts/02-verify.py
          python3 docs/experiments/recall-selection/scripts/05-verify-archive.py ;;
  *) python3 docs/experiments/recall-selection/scripts/01-run.py "$1" ;;
esac
