#!/bin/sh
# 사용: ./run.sh collect <claude|codex> <formal|pilot> | process | analyze | verify
set -eu
cd "$(dirname "$0")/scripts"
case "${1:-}" in
  collect) python3 01-collect.py "${2:?provider}" "${3:?formal|pilot}" ;;
  process) python3 02-process.py ;;
  analyze) python3 02-process.py && python3 03-analyze.py ;;
  verify) python3 02-process.py --check && python3 03-analyze.py --check ;;
  *) echo "사용: ./run.sh collect <claude|codex> <formal|pilot> | process | analyze | verify" >&2; exit 2 ;;
esac
