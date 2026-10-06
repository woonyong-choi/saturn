"""실제 채점기에 정답·오답·검사 변조·결측을 넣어 거짓 통과를 검사한다."""

from __future__ import annotations

import copy
import gzip
import json
import tempfile
from pathlib import Path

import analyze
import collect


def main() -> None:
    root = collect.REPO / ".runtime"
    with tempfile.TemporaryDirectory(prefix="grade587-", dir=root) as temp:
        base = Path(temp) / "case"
        facts, _, tests = collect.build(base, 9902)
        config = base / "practice/app/config.py"
        config.write_text(
            f"REQUEST_HEADER = {facts['header_new']!r}\nTIMEOUT_SECONDS = {facts['timeout']}\nRETRIES = 2\n"
        )
        report = base / "practice/app/report.py"
        report.write_text(
            collect.fixtures.REPORT.replace('"unknown"', repr(facts["slow_endpoint"]))
        )
        result = collect.grade(base, facts, tests)
        checked = (
            "header_ok",
            "timeout_ok",
            "retries_ok",
            "slow_ok",
            "visible_tests_ok",
        )
        if not all(result[k] for k in checked):
            raise RuntimeError("correct fixture rejected")
        config.write_text(
            config.read_text().replace(
                f"TIMEOUT_SECONDS = {facts['timeout']}", "TIMEOUT_SECONDS = 0"
            )
        )
        result = collect.grade(base, facts, tests)
        if result["timeout_ok"] or result["visible_tests_ok"]:
            raise RuntimeError("wrong timeout accepted")
        test = base / "practice/tests/test_client.py"
        test.write_text("")
        if collect.grade(base, facts, tests)["tests_unchanged"]:
            raise RuntimeError("test removal accepted")
    rawfile = collect.EXP / "data/raw/claude-9901-rrf_lookup.json.gz"
    raw = json.load(gzip.open(rawfile, "rt"))
    original = analyze.trial_row(raw)
    changed = copy.deepcopy(raw)
    changed["store"]["usage"][-1]["input_tokens"] = None
    modified = analyze.trial_row(changed)
    if (
        original["missing_usage"]
        or not modified["missing_usage"]
        or modified["total_cost"] is not None
    ):
        raise RuntimeError("missing usage interpreted as zero")
    if analyze.paired_interval([0, 4, 0, 0])[1] >= 0:
        raise RuntimeError("tiny paired sample produces overconfident quality claim")
    split_raw = json.load(
        gzip.open(collect.EXP / "data/raw/claude-101-rescue.json.gz", "rt")
    )
    split_row = analyze.trial_row(split_raw)
    if split_row["protocol_valid"] or split_row["f2"] != 1:
        raise RuntimeError("split input falsely accepted or known ticket misgraded")
    absent = copy.deepcopy(split_raw)
    for event in absent["store"]["events"]:
        body = json.loads(event["body"])
        if "Text" in body:
            body["Text"]["text"] = ""
            event["body"] = json.dumps(body)
    if analyze.trial_row(absent)["f2"]:
        raise RuntimeError("ticket accepted without assistant answer")
    for name in ("claude-102-provider", "codex-103-provider"):
        provider_raw = json.load(
            gzip.open(collect.EXP / "data/raw" / (name + ".json.gz"), "rt")
        )
        provider_row = analyze.trial_row(provider_raw)
        if (
            not provider_row["native_compaction_usage_unresolved"]
            or provider_row["total_cost"] is not None
        ):
            raise RuntimeError("unmetered native compaction interpreted as free")
    result = {
        "correct_fixture": True,
        "wrong_timeout_rejected": True,
        "test_removal_rejected": True,
        "unknown_usage_not_zero": True,
        "small_sample_quality_not_proven": True,
        "split_input_detected_and_ticket_regraded": True,
        "absent_ticket_rejected": True,
        "unmetered_native_compaction_not_free": True,
    }
    (collect.EXP / "results/validation.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )
    print("eight independent controls passed")


if __name__ == "__main__":
    main()
