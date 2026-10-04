"""가림 함수 재사용과 비공개 저장·호출 상한을 담당한다."""

from __future__ import annotations

import datetime as dt
import fcntl
import hashlib
import importlib.util
import json
import os
import subprocess
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
WORKTREE = PUBLIC.parents[2]
PRIVATE = Path.home() / "workspace/oss/saturn/.local/experiments/continuation-ko"
SEED = 606
LIMITS = {"jev": 4000, "codex": 400}
MODELS = ("gpt-6-astra", "gpt-5.6-luna")
spec = importlib.util.spec_from_file_location(
    "previous_mask", PUBLIC.parent / "constraint-long-context/scripts/common.py"
)
previous = importlib.util.module_from_spec(spec)
spec.loader.exec_module(previous)


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()[:16]


def mask(text: str) -> str:
    key = os.environ.get("SATURN_JUDGE_KEY")
    return previous.mask_text(text.replace(key, "[secret]") if key else text)


def read(path: Path) -> Any:
    return json.loads(path.read_text())


def write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def rows(path: Path) -> list[dict]:
    return (
        [json.loads(line) for line in path.read_text().splitlines() if line]
        if path.exists()
        else []
    )


def append(path: Path, row: dict) -> None:
    with path.open("a") as out:
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
        out.flush()
        os.fsync(out.fileno())


def setup() -> None:
    subprocess.run(
        ["git", "check-ignore", "-q", ".local/experiments/continuation-ko/probe"],
        cwd=PRIVATE.parents[2],
        check=True,
    )
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    (PRIVATE / "runtime").mkdir(exist_ok=True)


def reserve(kind: str, trial_id: str) -> None:
    with (PRIVATE / "budget.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        calls = rows(PRIVATE / "calls.jsonl")
        if any(r["trial_id"] == trial_id for r in calls):
            raise RuntimeError("trial already reserved")
        count = sum(r["kind"] == kind for r in calls)
        if count >= LIMITS[kind]:
            raise RuntimeError("call limit reached")
        append(
            PRIVATE / "calls.jsonl",
            {"kind": kind, "trial_id": trial_id, "ordinal": count + 1, "ts_utc": now()},
        )
