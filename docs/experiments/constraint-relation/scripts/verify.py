"""상한·원본 불변·봉인·원응답 보존과 계산 경계를 검사한다."""

from __future__ import annotations

import datetime
import hashlib
import json
import os
import subprocess
from collections import Counter

from candidates import MAX_BYTES, make_trial
from metrics import classify, operation_counts
from support import LIMITS, PRIVATE, PUBLIC, read_json, read_rows, source_manifest


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def check_accounting() -> None:
    calls = read_rows(PRIVATE / "calls.jsonl")
    receipts = read_rows(PRIVATE / "jev.jsonl")
    ids = [r["trial_id"] for r in calls]
    require(len(ids) == len(set(ids)), "duplicate call reservation")
    counts = Counter(r["kind"] for r in calls)
    require(
        all(counts[k] <= limit for k, limit in LIMITS.items()), "call limit exceeded"
    )
    saved = [r["trial_id"] for r in receipts]
    require(len(saved) == len(set(saved)), "duplicate receipt")
    require(set(saved) <= set(ids), "receipt without reservation")
    for row in receipts:
        size = len(json.dumps(row["request"], ensure_ascii=False).encode())
        require(size <= MAX_BYTES, "request byte limit exceeded")
        require(size == row["meta"]["request_bytes"], "recorded request size mismatch")
    run = read_json(PRIVATE / "run.json")
    stamp = int(
        subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", run["design_commit"]],
            cwd=PUBLIC,
            text=True,
        )
    )
    require(
        all(
            datetime.datetime.fromisoformat(r["ts_utc"]).timestamp() >= stamp
            for r in calls
        ),
        "call before design commit",
    )
    require(
        source_manifest() == read_json(PRIVATE / "plan.json")["source_manifest"],
        "source changed",
    )
    require(
        counts == Counter(read_json(PUBLIC / "results/summary.json")["calls"]),
        "summary call mismatch",
    )


def check_boundaries() -> None:
    require(classify([0.95, 0.85], 0.8) == "release", "release priority")
    require(classify([None, 0.99], 0.8) == "invalid", "invalid answer emitted action")
    row = {"gold": "replace", "probabilities": [[None, None]] * 3, "measured": False}
    metric = operation_counts([row], "replace", 0.8)
    require(
        metric["recall"]["n"] == 1 and metric["recall"]["k"] == 0,
        "missing target omitted from recall",
    )
    require(metric["precision"]["n"] == 0, "missing target counted as prediction")
    turn = {"text": "a" * 100001, "turn_index": 1, "turn_id": "u-0002"}
    meta = {
        "cohort": "original",
        "condition": "C1",
        "conversation_id": "fixture",
        "turn_id": "u-0002",
    }
    trial = make_trial(
        turn, [{"text": "a", "turn_index": 0, "turn_id": "u-0001"}], meta
    )
    require(trial["status"] == "oversize_input", "oversize input sent")
    try:
        require(False, "negative verifier fixture")
    except AssertionError:
        return
    raise AssertionError("negative fixture passed")


def check_secret() -> None:
    key = os.environ.get("SATURN_JUDGE_KEY")
    if not key:
        return
    for root in (PRIVATE, PUBLIC):
        for path in root.rglob("*"):
            if path.is_file():
                require(
                    key.encode() not in path.read_bytes(), "secret found in artifact"
                )


def write_checksums() -> None:
    paths = [
        PRIVATE / "plan.json",
        PRIVATE / "run.json",
        PRIVATE / "calls.jsonl",
        PRIVATE / "jev.jsonl",
    ]
    paths += sorted((PRIVATE / "extension").glob("*.json"))
    paths += sorted((PRIVATE / "codex").glob("*/receipt.json"))
    lines = [
        hashlib.sha256(p.read_bytes()).hexdigest() + "  " + str(p.relative_to(PRIVATE))
        for p in paths
    ]
    (PUBLIC / "data/SHA256SUMS").write_text("\n".join(lines) + "\n")


def main() -> None:
    check_accounting()
    check_boundaries()
    check_secret()
    write_checksums()
    print(
        "verified: source hashes, call caps, unique receipts, request bytes, design seal, metric boundaries"
    )


if __name__ == "__main__":
    main()
