"""앞 실험은 읽기만 하고 새 실행의 경로와 호출 장부를 관리한다."""

from __future__ import annotations

import fcntl
import hashlib
import importlib
import json
import os
import sys
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/constraint-relation"
SOURCE = Path.home() / "workspace/oss/saturn/.local/experiments/constraint-deep"
sys.path.append(str(PUBLIC.parent / "constraint-deep/scripts"))
legacy = importlib.import_module("storage")
labeler = importlib.import_module("labels")
judge = importlib.import_module("judge")
statistics_metrics = importlib.import_module("statistics_metrics")
ratio = statistics_metrics.ratio
percentile = statistics_metrics.percentile
binomial_tail = statistics_metrics.binomial_tail

SEED = 382
LIMITS = {"jev": 4000, "codex": 300}
read_json = legacy.read_json
read_rows = legacy.read_rows
now = legacy.now


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    key = os.environ.get("SATURN_JUDGE_KEY")
    path.write_text(text.replace(key, "[secret]") if key else text)


def append_row(path: Path, value: dict) -> None:
    key = os.environ.get("SATURN_JUDGE_KEY")
    text = json.dumps(value, ensure_ascii=False)
    with path.open("a") as stream:
        stream.write((text.replace(key, "[secret]") if key else text) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def reserve_call(kind: str, trial_id: str) -> int:
    with (PRIVATE / "budget.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        rows = read_rows(PRIVATE / "calls.jsonl")
        if any(r["trial_id"] == trial_id for r in rows):
            raise RuntimeError("trial already reserved")
        count = sum(r["kind"] == kind for r in rows)
        if count >= LIMITS[kind]:
            raise RuntimeError("call limit reached")
        append_row(
            PRIVATE / "calls.jsonl",
            {"kind": kind, "trial_id": trial_id, "ordinal": count + 1, "ts_utc": now()},
        )
        return count + 1


def initialize() -> None:
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    labeler.PRIVATE = PRIVATE
    labeler.reserve_call = reserve_call
    judge.reserve_call = reserve_call


def load_original() -> tuple[list[dict], dict, dict]:
    selected = read_json(SOURCE / "selection.json")["selected"]
    conversations = [
        read_json(SOURCE / "conversations" / (m["conversation_id"] + ".json"))
        for m in selected
    ]
    labels = {}
    for c in conversations:
        cid = c["conversation_id"]
        labels[cid] = {
            r["turn_id"]: r
            for chunk in read_rows(SOURCE / "labels/adjudicated" / (cid + ".jsonl"))
            for r in chunk["turns"]
        }
    receipts = {r["trial_id"]: r for r in read_rows(SOURCE / "jev.jsonl")}
    return conversations, labels, receipts


def source_manifest() -> dict:
    paths = [SOURCE / "selection.json", SOURCE / "census.json", SOURCE / "jev.jsonl"]
    paths += sorted((SOURCE / "conversations").glob("*.json"))
    paths += sorted((SOURCE / "labels").rglob("*.jsonl"))
    return {
        str(p.relative_to(SOURCE)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in paths
    }
