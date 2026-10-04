"""비밀값 가림·호출 상한·상태 재생·통계 경계를 검증한다."""

from __future__ import annotations

import ast
from pathlib import Path

from judge import parse_answers
from labels import replay_labels, validate_labels
from statistics_metrics import ratio
from storage import LIMITS, PRIVATE, PUBLIC, mask_text, read_rows


def main() -> None:
    for path in Path(__file__).parent.glob("*.py"):
        ast.parse(path.read_text())
    cases = [
        ("sk-proj-" + "a" * 24, "[secret]"),
        ("person@example.org", "[email]"),
        ("/Users/person/private/file.rs", "[abs]/file.rs"),
    ]
    for text, expected in cases:
        if mask_text(text) != expected:
            raise AssertionError("masking contract failed")
    old = {
        "turn_id": "u-1",
        "is_constraint": True,
        "operation": "register",
        "targets": [],
        "scope": "global",
        "reason": "persistent",
        "ambiguous": False,
    }
    release = {
        **old,
        "turn_id": "u-2",
        "is_constraint": False,
        "operation": "release",
        "targets": ["u-1"],
    }
    if replay_labels(set(), [old, release]):
        raise AssertionError(
            "release without new constraint must remove old constraint"
        )
    valid = {"turns": [old, release], "final_active_turn_ids": []}
    validate_labels(valid, ["u-1", "u-2"], set())
    try:
        validate_labels(
            {**valid, "final_active_turn_ids": ["u-1"]}, ["u-1", "u-2"], set()
        )
    except ValueError:
        pass
    else:
        raise AssertionError("inconsistent final set accepted")
    if ratio(46, 93)["half_width"] > 0.1 or ratio(46, 92)["half_width"] <= 0.1:
        raise AssertionError("Wilson precision sample boundary failed")
    for value in (float("nan"), True, 1.1, -0.1):
        if (
            parse_answers({"answers": {"q": {"noul": value}}}, {"q": {}})["q"]
            is not None
        ):
            raise AssertionError("invalid probability accepted")
    calls = read_rows(PRIVATE / "calls.jsonl")
    if len({row["trial_id"] for row in calls}) != len(calls):
        raise AssertionError("duplicate reservation")
    for kind, limit in LIMITS.items():
        if sum(row["kind"] == kind for row in calls) > limit:
            raise AssertionError("call budget exceeded")
    for path in PUBLIC.rglob("*"):
        if path.is_file() and path.stat().st_size > 50_000_000:
            raise AssertionError("public file size limit exceeded")
    print(
        "verification passed: masking, release replay, invalid answers, Wilson boundary, budgets"
    )


if __name__ == "__main__":
    main()
