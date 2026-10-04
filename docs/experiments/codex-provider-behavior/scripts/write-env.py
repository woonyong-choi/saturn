#!/usr/bin/env python3
"""실행 환경을 env.json에 쓴다. 수집을 마친 뒤 한 번 실행한다."""
import glob
import json
import os
import platform
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]


def run(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True, errors="replace").stdout.strip()


runs = sorted({Path(p).name.split("-calls")[0] for p in glob.glob(str(ROOT / "data" / "raw" / "*-calls.jsonl"))})
calls = sum(len(Path(p).read_text().splitlines()) for p in glob.glob(str(ROOT / "data" / "raw" / "*-calls.jsonl")))
env = {
    "os": f"{platform.system()} {platform.release()}",
    "machine": platform.machine(),
    "cpu": run("sysctl", "-n", "machdep.cpu.brand_string"),
    "memory_gb": round(int(run("sysctl", "-n", "hw.memsize")) / 2**30),
    "python": platform.python_version(),
    "codex_cli": run("codex", "--version"),
    "cargo": run("cargo", "--version"),
    "git": run("git", "--version"),
    "model": "gpt-5.6-luna",
    "execution_date": "2026-10-04",
    "design_commit": run("git", "-C", str(WORKTREE), "log", "--diff-filter=A", "--format=%H", "--", "docs/experiments/codex-provider-behavior/design.md").splitlines()[-1],
    "run_ids": runs,
    "codex_model_calls_total": calls,
    "codex_model_call_limit": 120,
    "auth": "전용 CODEX_HOME에 auth.json 심볼릭 링크만 만들고 원본은 읽지 않았다",
    "keychain": "가짜 서비스 saturn-test-dummy만 만들고 삭제했다",
    "seed": "해당 없음: 무작위 배정 없음, 모델 샘플링은 고정하지 않음",
}
(ROOT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(json.dumps(env, ensure_ascii=False, indent=2))
