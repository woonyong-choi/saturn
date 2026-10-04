"""요청 계약, 통계 경계, 호출 상한과 원자료 완전성을 검사한다."""

from __future__ import annotations

import importlib.util
import math
import json
import hashlib
import subprocess
from types import ModuleType
from collections import Counter

from metrics import ratio, confusion, measure, mcnemar
from storage import LIMITS, PRIVATE, PUBLIC, WORKTREE, read, rows


def module(name: str, filename: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, PUBLIC / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def main() -> None:
    judge = module("judge", "03-judge.py")
    source = (WORKTREE / "saturn-terminal/core/src/routers/mod.rs").read_text()
    source = source.replace("\\\n                 ", "")
    for text in (judge.KEEP, judge.ACTIONABLE, judge.RELATION, judge.SEND):
        if text not in source:
            raise AssertionError("question differs from engine")
    case = {
        "running": True,
        "input": "추가 확인",
        "previous_input": "수정",
        "goal_excerpt": "수정",
        "progress_excerpt": "검사 중",
    }
    a, b = judge.request_for(case, "A"), judge.request_for(case, "B")
    if (
        not b["state"].startswith(a["state"] + "\nprevious task context: ")
        or a["questions"] != b["questions"]
    ):
        raise AssertionError("condition contrast mismatch")
    if len(a["questions"]) != 4 or a["questions"]["relation_to_running"][
        "criteria"
    ] != dict.fromkeys(judge.RELATION_OPTIONS):
        raise AssertionError("question set mismatch")
    case["running"] = False
    if len(judge.request_for(case, "A")["questions"]) != 2:
        raise AssertionError("idle question set mismatch")
    try:
        judge.parse(
            {"answers": {"keep_current": {"noul": math.nan}}},
            {"keep_current": {"type": "noul"}},
        )
    except ValueError:
        pass
    else:
        raise AssertionError("nan accepted")
    if ratio(0, 0)["value"] is not None or ratio(0, 10)["ci"][0] != 0:
        raise AssertionError("empty or boundary interval mismatch")
    if not math.isclose(ratio(5, 10)["ci"][0], 0.23659309051256405):
        raise AssertionError("wilson reference mismatch")
    if confusion([(True, True), (False, True), (True, False), (False, False)]) != [
        1,
        1,
        1,
        1,
    ]:
        raise AssertionError("confusion matrix mismatch")
    if measure([2, 1, 1, 2])["f1"] != 2 / 3 or mcnemar(10, 0) != 0.001953125:
        raise AssertionError("metrics reference mismatch")
    counts = Counter(r["kind"] for r in rows(PRIVATE / "calls.jsonl"))
    if any(counts[k] > limit for k, limit in LIMITS.items()):
        raise AssertionError("call budget exceeded")
    receipts = rows(PRIVATE / "jev.jsonl")
    if len({r["trial_id"] for r in receipts}) != len(receipts):
        raise AssertionError("duplicate receipts")
    if (PRIVATE / "sample.json").exists():
        sample = read(PRIVATE / "sample.json")
        if len(sample) > 660 or len({r["id"] for r in sample}) != len(sample):
            raise AssertionError("sample coverage mismatch")
        if not any(r["is_saturn"] for r in sample):
            raise AssertionError("saturn missing")
    summary_path = PUBLIC / "results/summary.json"
    if summary_path.exists() and (PRIVATE / "sample.json").exists():
        summary = read(summary_path)
        sample = read(PRIVATE / "sample.json")
        expected = {
            f"{case['id']}-{condition}-r{repeat}"
            for case in sample
            for condition in ("A", "B")
            for repeat in (1, 2, 3)
        }
        if {r["trial_id"] for r in receipts} != expected:
            raise AssertionError("missing repeat receipts")
        for call in rows(PRIVATE / "calls.jsonl"):
            if (
                call["kind"] == "codex"
                and not (PRIVATE / "codex" / call["trial_id"] / "receipt.json").exists()
            ):
                raise AssertionError("reserved label call has no receipt")
        if summary["calls"] != dict(counts):
            raise AssertionError("call counts differ from journal")
        for name in ("gpt-6-astra", "gpt-5.6-luna"):
            labels = read(PRIVATE / f"labels-{name}.json")
            if {r["id"] for r in labels} != {r["id"] for r in sample}:
                raise AssertionError("label coverage mismatch")
        for row in receipts:
            if row["status"] == "ok":
                judge.parse(
                    json.loads(row["raw_response"]), row["request"]["questions"]
                )
        for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
            checksum, name = line.split("  ", 1)
            if hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() != checksum:
                raise AssertionError("private input checksum mismatch")
        frozen = subprocess.check_output(
            [
                "git",
                "show",
                summary["run"]["design_commit"]
                + ":docs/experiments/continuation-judgment-korean/design.md",
            ],
            cwd=WORKTREE,
        )
        if frozen != (PUBLIC / "design.md").read_bytes():
            raise AssertionError("sealed design changed")
        if len(read(PRIVATE / "human-review.json")["cases"]) != 30:
            raise AssertionError("human review coverage mismatch")
    print("verify passed")


if __name__ == "__main__":
    main()
