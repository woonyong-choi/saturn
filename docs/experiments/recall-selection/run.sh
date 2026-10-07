#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
python3 docs/experiments/recall-selection/scripts/01-run.py "${1:-analyze}"
