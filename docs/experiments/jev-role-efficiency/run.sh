#!/bin/sh
set -eu
cd "$(dirname "$0")"
check_saved_inputs() {
  python3 - <<'PY'
from pathlib import Path
import hashlib
public = Path.cwd()
private = public.parents[2] / '.local/experiments/jev-role-efficiency'
for name in ('SHA256SUMS', 'INPUT-SHA256SUMS'):
    manifest = public / 'data' / name
    if manifest.exists():
        for line in manifest.read_text().splitlines():
            expected, relative = line.split('  ', 1)
            if hashlib.sha256((private / relative).read_bytes()).hexdigest() != expected:
                raise SystemExit('saved input hash mismatch: ' + relative)
PY
}
check_saved_inputs
case "${1:-}" in
  collect) python3 scripts/01-collect.py ;;
  process|analyze) python3 scripts/02-analyze.py; python3 scripts/diagnostics/04-audit.py ;;
  verify) python3 scripts/03-verify.py ;;
  all) python3 scripts/01-collect.py; python3 scripts/02-analyze.py; python3 scripts/diagnostics/04-audit.py; python3 scripts/03-verify.py ;;
  *) echo 'usage: run.sh collect|process|analyze|verify|all' >&2; exit 2 ;;
esac
