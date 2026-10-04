"""호출 예약과 키 가림을 앞 실험의 실행 경계와 공유한다."""

from __future__ import annotations

import importlib.util
import os
import subprocess
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
MAIN = Path.home() / "workspace/oss/saturn"
PRIVATE = MAIN / ".local/experiments/constraint-scope"
PRIOR = MAIN / ".local/experiments/constraint-model-compare"
SEED = 382104
LIMITS = {"codex": 800, "jev": 4000}


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BASE = load_module(
    "previous_runtime", PUBLIC.parent / "constraint-model-compare/scripts/runtime.py"
)
BASE.PRIVATE, BASE.ROOT, BASE.LIMITS = PRIVATE, ROOT, LIMITS
read, write, rows = BASE.read, BASE.write, BASE.rows
redact, reserve, now = BASE.redact, BASE.reserve, BASE.now
call_cli, response_text = BASE.call_cli, BASE.response_text
MODELS = {"astra": "gpt-6-astra", "sol": "gpt-6-sol"}
QUESTION = "Does the user's latest input set a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do?"


# cost: io 1 ignore check; basis: estimate
def setup() -> None:
    subprocess.run(
        ["git", "check-ignore", "-q", ".local/experiments/constraint-scope/probe"],
        cwd=MAIN,
        check=True,
    )
    os.umask(0o077)
    for path in (PRIVATE / "raw", PRIVATE / "runtime/cache"):
        path.mkdir(parents=True, exist_ok=True)
