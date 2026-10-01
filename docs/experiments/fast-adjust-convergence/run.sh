#!/usr/bin/env bash
# 빠른 조정 수렴 실험: collect, process, analyze, verify, all
set -euo pipefail

cd "$(dirname "$0")"
PYTHON="${PYTHON:-python3}"

collect() {
  local commit ts run_id
  commit="$(git rev-parse --short=7 HEAD)"
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  run_id="$(date -u +%Y%m%dT%H%M%SZ)-${commit}"
  mkdir -p data/raw
  "$PYTHON" scripts/01-collect.py --out-dir data/raw --run-id "$run_id" --ts-utc "$ts"
  write_env "$run_id" "$commit" "$ts"
  (cd data && shasum -a 256 raw/* > SHA256SUMS)
}

write_env() {
  "$PYTHON" - "$@" <<'PY'
import json, os, platform, subprocess, sys
import numpy

run_id, commit, ts = sys.argv[1:4]
def sysctl(key):
    try:
        return subprocess.run(["sysctl", "-n", key], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
memory = sysctl("hw.memsize")
env = {
    "os": f"{platform.system()} {platform.release()}",
    "cpu": sysctl("machdep.cpu.brand_string") or platform.processor(),
    "memory_gb": round(int(memory) / 2**30) if memory else None,
    "tools": {"python": platform.python_version(), "numpy": numpy.__version__},
    "model": "해당 없음",
    "seed": 120,
    "run_id": run_id,
    "run_date_utc": ts,
    "commit": commit,
}
with open("env.json", "w", encoding="utf-8") as file:
    json.dump(env, file, ensure_ascii=False, indent=2)
    file.write("\n")
PY
}

process() {
  "$PYTHON" scripts/02-process.py --raw-dir data/raw --out-dir data/processed
}

analyze() {
  process
  "$PYTHON" scripts/03-analyze.py --processed-dir data/processed --out-dir results
}

verify() {
  (cd data && shasum -a 256 -c SHA256SUMS)
}

case "${1:-}" in
  collect) collect ;;
  process) process ;;
  analyze) analyze ;;
  verify) verify ;;
  all) collect && verify && analyze ;;
  *) echo "usage: $0 {collect|process|analyze|verify|all}" >&2; exit 2 ;;
esac
