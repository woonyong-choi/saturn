"""실험 경로, 입력 가림과 호출 전 예산 예약을 관리한다."""

from __future__ import annotations

import fcntl
import importlib
import json
import os
import sys
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/constraint-cancel"
SOURCE = Path.home() / "workspace/oss/saturn/.local/experiments/constraint-deep"
sys.path.append(str(PUBLIC.parent / "constraint-deep/scripts"))
legacy = importlib.import_module("storage")
labeler = importlib.import_module("labels")
judge = importlib.import_module("judge")
read_json = legacy.read_json
read_rows = legacy.read_rows
now = legacy.now
SEED = 38201
LIMITS = {"jev": 5000, "codex": 400}
POSITIVE = (
    "direct",
    "deictic",
    "content",
    "number",
    "partial",
    "conditional",
    "indirect",
    "mixed",
)
NEGATIVE = ("task", "keyword", "stop", "strengthen", "new_rule")


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    key = os.environ.get("SATURN_JUDGE_KEY")
    path.write_text(text.replace(key, "[secret]") if key else text)


def append_row(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(value, ensure_ascii=False)
    key = os.environ.get("SATURN_JUDGE_KEY")
    with path.open("a") as stream:
        stream.write((text.replace(key, "[secret]") if key else text) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


# cost: io 1 locked journal append; basis: estimate
def reserve_call(kind: str, trial_id: str) -> int:
    with (PRIVATE / "budget.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        rows = read_rows(PRIVATE / "calls.jsonl")
        if any(r["trial_id"] == trial_id for r in rows):
            raise RuntimeError("trial already reserved; do not resend")
        count = sum(r["kind"] == kind for r in rows)
        if count >= LIMITS[kind]:
            raise RuntimeError("call budget reached")
        append_row(
            PRIVATE / "calls.jsonl",
            dict(kind=kind, trial_id=trial_id, ordinal=count + 1, ts_utc=now()),
        )
        return count + 1


def initialize() -> None:
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    labeler.PRIVATE = PRIVATE
    labeler.reserve_call = reserve_call
    judge.reserve_call = reserve_call


def codex(model: str, prompt: str, trial: str, schema: dict) -> dict:
    receipt = PRIVATE / "codex" / trial / "receipt.json"
    if receipt.exists():
        r = read_json(receipt)
        if r["returncode"] != 0:
            raise RuntimeError("stored codex failure")
        text = r["stdout"]
        return json.loads(text[text.find("{") : text.rfind("}") + 1])
    return labeler.call_codex(model, prompt, trial, schema)


def schema_rows(properties: dict) -> dict:
    return {
        "type": "object",
        "additionalProperties": False,
        "required": ["rows"],
        "properties": {
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": list(properties),
                    "properties": properties,
                },
            }
        },
    }


def state(item: dict, record: bool) -> dict:
    result = {
        "active_constraints": [
            {"id": f"c{i + 1}", "rule": r, "registration_order": i + 1}
            for i, r in enumerate(item["rules"])
        ],
        "previous_context": item.get("context", []),
        "latest_user_input": item["text"],
    }
    if record:
        result["chat_record"] = [f"제약 등록됨 · {r}" for r in item["rules"]]
    return result
