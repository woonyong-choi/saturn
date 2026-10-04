"""비밀값 가림·호출 상한·상태 재생·통계 경계를 검증한다."""

from __future__ import annotations

import ast
import datetime as dt
import subprocess
from pathlib import Path
from unittest.mock import patch

from analysis_metrics import classify_relation, flatten_label_chunks
from judge import parse_answers
from labels import _run_validated, replay_labels, validate_labels
from statistics_metrics import metrics, ratio
from storage import LIMITS, PRIVATE, PUBLIC, mask_text, read_rows


# cost: time O(n), heap O(n), io local artifacts and 2 git processes; vars: n = saved calls; basis: estimate
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
    with patch.dict(
        "os.environ", {"SATURN_JUDGE_KEY": "synthetic-unrecognized-credential"}
    ):
        if mask_text("value synthetic-unrecognized-credential") != "value [secret]":
            raise AssertionError("exact environment key was not redacted")
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
    duplicated = {"turns": [old], "final_active_turn_ids": ["u-1", "u-1"]}
    with patch("labels._load_or_call", return_value=duplicated):
        retained = _run_validated("unused", "", "unused", ["u-1"], set())
    if retained.get("final_set_valid", True) or not retained.get("protocol_deviation"):
        raise AssertionError("invalid final set must remain flagged")
    if retained["final_active_turn_ids"] != duplicated["final_active_turn_ids"]:
        raise AssertionError("declared final set was silently rewritten")
    if ratio(46, 93)["half_width"] > 0.1 or ratio(46, 92)["half_width"] <= 0.1:
        raise AssertionError("Wilson precision sample boundary failed")
    for value in (float("nan"), True, 1.1, -0.1):
        if (
            parse_answers({"answers": {"q": {"noul": value}}}, {"q": {}})["q"]
            is not None
        ):
            raise AssertionError("invalid probability accepted")
    for invalid in ({"turns": [None]}, {"turns": None}, []):
        try:
            validate_labels(invalid, ["u-1"], set())
        except ValueError:
            continue
        raise AssertionError("invalid JSON structure must reach the repair handler")
    rows = flatten_label_chunks(
        [
            {"turns": [{"turn_id": "a"}]},
            {"turns": [{"turn_id": "b"}], "protocol_deviation": True},
            {"turns": [{"turn_id": "c"}]},
        ]
    )
    if [row["protocol_deviation"] for row in rows.values()] != [False, True, True]:
        raise AssertionError("dependent labels reentered confirmatory analysis")
    if classify_relation(0.9, 0.9) != "release":
        raise AssertionError("release must take precedence over replacement")
    if any(
        classify_relation(replace, release) != "invalid"
        for replace, release in ((None, None), (None, 0.9), (0.9, None))
    ):
        raise AssertionError("missing relationship response is not compatibility")
    confusion = metrics(
        [
            {"gold": gold, "probability": p}
            for gold, p in [(True, 0.9), (True, 0.2), (False, 0.9), (False, 0.1)]
        ],
        0.8,
    )
    if [confusion[name] for name in ("tp", "fp", "fn", "tn")] != [1, 1, 1, 1]:
        raise AssertionError("classification denominators failed")
    calls = read_rows(PRIVATE / "calls.jsonl")
    if len({row["trial_id"] for row in calls}) != len(calls):
        raise AssertionError("duplicate reservation")
    for kind, limit in LIMITS.items():
        if sum(row["kind"] == kind for row in calls) > limit:
            raise AssertionError("call budget exceeded")
    design_path = "docs/experiments/constraint-deep/design.md"
    sealed = subprocess.check_output(
        ["git", "show", "8b7c236:" + design_path], cwd=PUBLIC.parents[2]
    )
    if sealed != (PUBLIC / "design.md").read_bytes():
        raise AssertionError("sealed design changed")
    committed = int(
        subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "8b7c236"], cwd=PUBLIC.parents[2]
        )
    )
    if any(
        dt.datetime.fromisoformat(row["ts_utc"]).timestamp() < committed
        for row in calls
    ):
        raise AssertionError("model call predates seal")
    for path in PUBLIC.rglob("*"):
        if path.is_file() and path.stat().st_size > 50_000_000:
            raise AssertionError("public file size limit exceeded")
    print(
        "verification passed: masking, release replay, invalid answers, Wilson boundary, budgets"
    )


if __name__ == "__main__":
    main()
