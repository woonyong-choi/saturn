"""비공개 실행 경로와 내구성 있는 호출 장부를 관리한다."""

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
MAIN = Path("/Users/woonyong/workspace/oss/saturn")
PRIVATE = MAIN / ".local/experiments/constraint-deep"
SEED = 382
BANDS = ("10-29", "30-119", "120-399", "400+")
MODELS = ("gpt-6-astra", "gpt-5.6-luna")
LIMITS = {"jev": 8000, "codex": 600}
_SPEC = importlib.util.spec_from_file_location(
    "previous_masking", PUBLIC.parent / "constraint-long-context/scripts/common.py"
)
previous = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(previous)
mask_text = previous.mask_text
is_human_user_row = previous.is_human_user_row
get_text = previous.get_text


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()[:16]


def band_for(count: int) -> str | None:
    for upper, name in zip((29, 119, 399, float("inf")), BANDS):
        if count < 10:
            return None
        if count <= upper:
            return name
    return None


def read_json(path: Path) -> Any:
    return json.loads(path.read_text())


def read_rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line]


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def append_row(path: Path, row: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as stream:
        stream.write(json.dumps(row, ensure_ascii=False) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


# cost: io 1 process; basis: estimate
def ensure_private() -> None:
    result = subprocess.run(
        ["git", "check-ignore", "-q", ".local/experiments/constraint-deep/probe"],
        cwd=MAIN,
        check=False,
    )
    if result.returncode:
        raise RuntimeError("private directory is not ignored")
    PRIVATE.mkdir(parents=True, exist_ok=True)
    os.chmod(PRIVATE, 0o700)
    os.umask(0o077)


def reserve_call(kind: str, trial_id: str) -> int:
    with (PRIVATE / "budget.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        rows = read_rows(PRIVATE / "calls.jsonl")
        if any(row["trial_id"] == trial_id for row in rows):
            raise RuntimeError(
                "trial already reserved; inspect its receipt before resuming"
            )
        count = sum(row["kind"] == kind for row in rows)
        if count >= LIMITS[kind]:
            raise RuntimeError(f"{kind} call limit reached")
        append_row(
            PRIVATE / "calls.jsonl",
            {
                "kind": kind,
                "trial_id": trial_id,
                "ordinal": count + 1,
                "ts_utc": now(),
            },
        )
    return count + 1
