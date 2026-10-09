#!/bin/sh
set -eu
cd "$(dirname "$0")"
case "${1:-}" in
  collect)
    if [ -t 0 ]; then
      stty -echo
      trap 'stty echo' EXIT HUP INT TERM
    fi
    node scripts/01-collect.mjs "${2:-48}"
    ;;
  process|analyze)
    python3 scripts/02-process.py
    ;;
  verify)
    shasum -a 256 -c data/SHA256SUMS
    python3 scripts/02-process.py
    python3 - <<'PY'
import json
import hashlib
from pathlib import Path
summary = json.loads(Path('results/summary.json').read_text())
env = json.loads(Path('env.json').read_text())
raw = Path('../../../.local/experiments/context-compaction-comparison/sessions-20261005T115947Z-145293d.jsonl')
assert hashlib.sha256(raw.read_bytes()).hexdigest() == env['raw_sha256']
assert b'apikey_' not in raw.read_bytes()
assert summary['scenarios'] == 48
assert summary['questions_per_condition'] == 480
assert not summary['errors']
print('48 scenarios, 480 paired questions, raw hash and secret check passed')
PY
    ;;
  all)
    "$0" collect 48
    "$0" analyze
    "$0" verify
    ;;
  *)
    echo 'usage: ./run.sh {collect|process|analyze|verify|all}' >&2
    exit 2
    ;;
esac
