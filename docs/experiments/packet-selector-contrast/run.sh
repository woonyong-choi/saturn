#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."/..
script=docs/experiments/packet-selector-contrast/scripts
case "${1:-}" in
  seal) python3 "$script/01-collect.py" seal ;;
  collect) python3 "$script/01-collect.py" collect ;;
  process|analyze) python3 "$script/02-process.py" ;;
  verify) python3 "$script/03-verify.py" ;;
  all) python3 "$script/01-collect.py" collect; python3 "$script/02-process.py"; python3 "$script/03-verify.py" ;;
  *) echo "usage: $0 {seal|collect|process|analyze|verify|all}" >&2; exit 2 ;;
esac
