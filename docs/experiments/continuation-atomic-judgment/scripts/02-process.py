#!/usr/bin/env python3
"""두 질문의 원응답과 기존 B 첫 응답을 개발 라벨에 연결한다."""

from __future__ import annotations

import csv
import hashlib
import json
import math
import os
import sys
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[2]
FIELDS = ("id", "session", "label", "old_action", "new_action", "status", "old_tokens", "new_tokens", "elapsed_ms")


def tokens(value: dict | None) -> int | None:
    if not isinstance(value, dict):
        return None
    usage = value.get("usage")
    if not isinstance(usage, dict):
        return None
    a, b = usage.get("input_tokens"), usage.get("output_tokens")
    return a + b if type(a) is int and type(b) is int else None


def probability(value: object) -> float | None:
    if not isinstance(value, dict) or value.get("type") != "noul":
        return None
    result = value.get("noul")
    return float(result) if type(result) in (int, float) and math.isfinite(result) and 0 <= result <= 1 else None


def action(response: dict | None) -> str:
    if not isinstance(response, dict):
        return "ask"
    answers = response.get("answers")
    if not isinstance(answers, dict) or set(answers) != {"same_goal", "independent_goal"}:
        return "ask"
    same, independent = probability(answers["same_goal"]), probability(answers["independent_goal"])
    if same is None or independent is None:
        return "ask"
    if same >= 0.7 and independent <= 0.3:
        return "continue"
    if independent >= 0.7 and same <= 0.3:
        return "new"
    return "ask"


def rows(run_id: str) -> tuple[list[dict], list[str]]:
    source = Path(os.environ["SATURN_CONTINUATION_LABELS"])
    old_source = Path(os.environ["SATURN_CONTINUATION_OLD_JEV"])
    run = ROOT / ".runtime" / "continuation-atomic-judgment" / run_id
    manifest = json.loads((run / "manifest.json").read_text(encoding="utf-8"))
    sample = Path(os.environ["SATURN_CONTINUATION_SAMPLE"])
    if manifest["sample_sha256"] != hashlib.sha256(sample.read_bytes()).hexdigest():
        raise RuntimeError("source sample changed")
    labels = json.loads(source.read_text(encoding="utf-8"))
    if len(labels) != 608 or len({case["id"] for case in labels}) != 608:
        raise RuntimeError("labels changed")
    old = {}
    for line in old_source.read_text(encoding="utf-8").splitlines():
        item = json.loads(line)
        if item["condition"] == "B" and item["repeat"] == 1:
            old[item["id"]] = item
    result = []
    hashes = [f"{hashlib.sha256((run / 'manifest.json').read_bytes()).hexdigest()}  manifest.json"]
    for case in labels:
        name = case["id"]
        response_path = run / f"{name}.json"
        reservation = run / f"{name}.reservation.json"
        if reservation.exists():
            hashes.append(f"{hashlib.sha256(reservation.read_bytes()).hexdigest()}  {name}.reservation.json")
        old_item = old.get(name, {})
        old_probability = old_item.get("answers", {}).get("keep_current")
        old_action = "continue" if type(old_probability) in (int, float) and old_probability >= 0.5 else "new"
        row = dict(id=name, session=case["session"], label=case["label"], old_action=old_action, new_action="ask", status="missing", old_tokens=tokens(json.loads(old_item["raw_response"])) if old_item.get("raw_response") else "", new_tokens="", elapsed_ms="")
        if response_path.exists():
            hashes.append(f"{hashlib.sha256(response_path.read_bytes()).hexdigest()}  {name}.json")
            raw = json.loads(response_path.read_text(encoding="utf-8"))
            row.update(status=raw["status"], new_action=action(raw["response"]), new_tokens=tokens(raw["response"]), elapsed_ms=raw["elapsed_ms"])
        result.append(row)
    return result, hashes


def main() -> None:
    run_id = sys.argv[1]
    result, hashes = rows(run_id)
    run = ROOT / ".runtime" / "continuation-atomic-judgment" / run_id
    with (run / "processed.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(result)
    data = BASE / "data"
    data.mkdir(exist_ok=True)
    (data / "SHA256SUMS").write_text("\n".join(hashes) + "\n", encoding="utf-8")
    print(f"processed {len(result)} rows")


if __name__ == "__main__":
    main()
