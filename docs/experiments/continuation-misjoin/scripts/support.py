"""앞 실험은 읽기만 하고 새 호출의 예산과 저장 경로를 제한한다."""

from __future__ import annotations

import datetime as dt
import fcntl
import hashlib
import importlib.util
import json
import os
import sys
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
WORKTREE = PUBLIC.parents[2]
PRIVATE = WORKTREE / ".local/experiments/continuation-misjoin"
BASE = Path.home() / "workspace/oss/saturn/.local/experiments/continuation-ko"
PRIOR = PUBLIC.parent / "continuation-judgment-korean"
SEED = 607
LIMITS = {"jev": 4000, "codex": 300}
CONDITIONS = ("B1", "B2", "B3")
sys.path.insert(0, str(PRIOR / "scripts"))


def load(name: str, filename: str) -> Any:
    spec = importlib.util.spec_from_file_location(name, PRIOR / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read(path: Path) -> Any:
    return json.loads(path.read_text())


def rows(path: Path) -> list[dict]:
    return (
        [json.loads(line) for line in path.read_text().splitlines()]
        if path.exists()
        else []
    )


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def append(path: Path, value: dict) -> None:
    with path.open("a") as out:
        out.write(json.dumps(value, ensure_ascii=False) + "\n")
        out.flush()
        os.fsync(out.fileno())


def setup() -> None:
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
