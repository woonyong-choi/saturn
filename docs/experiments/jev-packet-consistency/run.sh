#!/bin/sh
set -eu
cd "$(dirname "$0")"
command="${1:-}"
if [ "$#" -gt 0 ]; then shift; fi
repo_root="$(cd ../../.. && pwd)"
dev_raw="$repo_root/.runtime/jev-packet-consistency/dev-replay.jsonl"
dev_summary="$repo_root/.runtime/jev-packet-consistency/dev-summary.json"
if [ "${1:-}" = dev ]; then
  shift
  source_dir="${1:-}"
  if [ "$#" -gt 0 ]; then shift; fi
  if [ -z "$source_dir" ]; then echo 'development source directory required' >&2; exit 2; fi
  case "$command" in
    collect) python3 scripts/dev_replay.py collect --source "$source_dir" --key-file "${1:-}" --raw "$dev_raw" --summary "$dev_summary" ;;
    process|analyze) python3 scripts/dev_replay.py analyze --source "$source_dir" --raw "$dev_raw" --summary "$dev_summary" ;;
    verify) python3 scripts/dev_replay.py verify --source "$source_dir" --raw "$dev_raw" --summary "$dev_summary" ;;
    all)
      python3 scripts/dev_replay.py collect --source "$source_dir" --key-file "${1:-}" --raw "$dev_raw" --summary "$dev_summary"
      python3 scripts/dev_replay.py analyze --source "$source_dir" --raw "$dev_raw" --summary "$dev_summary"
      python3 scripts/dev_replay.py verify --source "$source_dir" --raw "$dev_raw" --summary "$dev_summary" ;;
    *) echo 'usage: ./run.sh collect|process|analyze|verify|all dev SOURCE_DIR [KEY_FILE]' >&2; exit 2 ;;
  esac
  exit 0
fi
case "$command" in
  collect|process|analyze|verify|all) python3 scripts/consistency.py "$command" "$@" ;;
  *) echo 'usage: ./run.sh collect|process|analyze|verify|all dev SOURCE_DIR [KEY_FILE], or command --phase confirm' >&2; exit 2 ;;
esac
