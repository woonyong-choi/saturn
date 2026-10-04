"""요청 계약, 통계 경계, 호출 상한과 원자료 완전성을 검사한다."""

from __future__ import annotations

import importlib.util
import math
from collections import Counter

from storage import LIMITS, PRIVATE, PUBLIC, WORKTREE, read, rows


def module(name: str, filename: str):
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
    print("verify passed")


if __name__ == "__main__":
    main()
