#!/bin/sh
# 사용: ./run.sh collect <claude|codex> <formal|pilot> | replay collect|analyze | process | analyze | verify
set -eu
cd "$(dirname "$0")/scripts"
case "${1:-}" in
  collect) python3 01-collect.py "${2:?provider}" "${3:?formal|pilot}" ;;
  replay) python3 04-replay.py "${2:?collect|analyze}" ;;
  process) python3 02-process.py ;;
  analyze) python3 02-process.py && python3 03-analyze.py ;;
  verify) (cd .. && shasum -a 256 -c data/SHA256SUMS >/dev/null) && python3 02-process.py --check && python3 03-analyze.py --check && python3 04-replay.py analyze --check ;;
  *) echo "사용: ./run.sh collect <claude|codex> <formal|pilot> | replay collect|analyze | process | analyze | verify" >&2; exit 2 ;;
esac
