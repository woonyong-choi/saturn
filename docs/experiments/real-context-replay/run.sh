#!/bin/sh
set -eu
STUDY=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$STUDY/../../.." && pwd)
export SATURN_REPLAY_HOME="${SATURN_REPLAY_HOME:-$REPO/.local/experiments/real-context-replay/corrected}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/cleanup-verification}"
PYTHON_EXEC="${SATURN_STUDY_PYTHON:-python3}"
case "${1:-verify}" in
  collect)
    if [ -e "$SATURN_REPLAY_HOME/calls-plan.json" ]; then
      echo 'collection exists; choose a fresh SATURN_REPLAY_HOME to preserve original responses' >&2
      exit 2
    fi
    if [ ! -f "$SATURN_REPLAY_HOME/cases.json" ]; then
      "$PYTHON_EXEC" "$STUDY/scripts/01-prepare.py"
    fi
    cd "$REPO"
    SATURN_REPLAY_INPUT="$SATURN_REPLAY_HOME/cases.json" \
    SATURN_REPLAY_OUTPUT="$SATURN_REPLAY_HOME/packets" \
      cargo test -p saturn-engine handoff::replay::export_real_record_packets -- --ignored
    "$PYTHON_EXEC" "$STUDY/scripts/02-collect.py"
    ;;
  process|analyze)
    "$PYTHON_EXEC" "$STUDY/scripts/03-analyze.py"
    ;;
  archive)
    "$PYTHON_EXEC" "$STUDY/scripts/05-archive.py"
    ;;
  verify)
    "$PYTHON_EXEC" "$STUDY/scripts/04-verify.py"
    ;;
  all)
    "$0" collect
    "$0" analyze
    "$0" archive
    "$0" verify
    ;;
  *) echo 'usage: run.sh collect|process|analyze|archive|verify|all' >&2; exit 2 ;;
esac
