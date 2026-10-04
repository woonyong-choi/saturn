#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
import math
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
RESULTS = ROOT / "results"
SUMMARY = RESULTS / "summary.json"
TABLE = RESULTS / "tables" / "conditions.csv"
ENV = ROOT / "env.json"


def rows() -> list[dict]:
    values = []
    for path in sorted(RAW.glob("*.jsonl")):
        with path.open(encoding="utf-8") as stream:
            values.extend(json.loads(line) for line in stream if line.strip())
    return values


def wilson(k: int, n: int) -> list[float]:
    if n == 0:
        return [None, None]
    z = 1.959963984540054
    p = k / n
    denominator = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / denominator
    half = z * math.sqrt((p * (1 - p) + z * z / (4 * n)) / n) / denominator
    return [round(max(0.0, centre - half), 6), round(min(1.0, centre + half), 6)]


def condition_result(condition: str, values: list[dict]) -> dict:
    observed = [
        row.get("resume_child_execution")
        if row.get("marker_start_count", 0) > 0
        and row.get("request_result") not in {"error_or_timeout", "timeout"}
        else None
        for row in values
    ]
    known = [value for value in observed if value is not None]
    k = sum(value is True for value in known)
    n = len(known)
    if len(known) != len(observed) or not observed:
        status = "cannot_distinguish"
    elif all(value is known[0] for value in known):
        status = "confirmed"
    else:
        status = "unstable"
    expected = {
        "codex.raw-resume": True,
        "codex.cleaned-resume": False,
        "claude.resume-env-present": True,
        "claude.resume-env-absent": False,
    }.get(condition)
    if expected is None:
        verdict = "탐색"
    elif status == "confirmed" and known[0] is expected:
        verdict = "확인"
    elif status == "confirmed":
        verdict = "기각"
    elif status == "unstable":
        verdict = "불안정"
    else:
        verdict = "확인 못 함"
    return {
        "condition": condition,
        "n": len(observed),
        "known_n": n,
        "rerun_count": k,
        "rerun_rate": round(k / n, 6) if n else None,
        "rerun_wilson_95": wilson(k, n),
        "trial_values": observed,
        "trial_ids": [row.get("trial_id") for row in values],
        "status": status,
        "verdict": verdict,
    }


def analyze() -> dict:
    grouped = defaultdict(list)
    all_rows = rows()
    # 같은 조건을 다시 수집한 실행이 있으면 그 실행의 행만 분석하고, 앞 실행은 따로 적는다.
    latest = {}
    for value in all_rows:
        latest[value["condition"]] = max(
            latest.get(value["condition"], ""), value["run_id"]
        )
    superseded = defaultdict(list)
    for value in all_rows:
        if value["run_id"] == latest[value["condition"]]:
            grouped[value["condition"]].append(value)
        else:
            superseded[value["condition"]].append(value)
    condition_results = [
        condition_result(condition, grouped[condition]) for condition in sorted(grouped)
    ]
    expected_conditions = [
        "codex.raw-resume",
        "codex.cleaned-resume",
        "claude.resume-env-present",
        "claude.resume-env-absent",
    ]
    hypotheses = []
    for index, condition in enumerate(expected_conditions, 1):
        result = next(
            (item for item in condition_results if item["condition"] == condition), None
        )
        hypotheses.append(
            {"hypothesis": f"H{index}", "condition": condition, "result": result}
        )
    exploratory = [
        item
        for item in condition_results
        if item["condition"] == "claude.task-subagent"
    ]
    environment = json.loads(ENV.read_text(encoding="utf-8"))
    return {
        "experiment": "crash-resume",
        "conditions": condition_results,
        "hypotheses": hypotheses,
        "exploratory": exploratory,
        "superseded_runs": [
            {
                "condition": condition,
                "run_id": run_id,
                "n": len(items),
                "request_results": [v.get("request_result") for v in items],
                "marker_start_counts": [v.get("marker_start_count") for v in items],
            }
            for condition, values in sorted(superseded.items())
            for run_id, items in sorted(
                {
                    key: [v for v in values if v["run_id"] == key]
                    for key in {v["run_id"] for v in values}
                }.items()
            )
        ],
        "model_calls": environment.get("model_calls", {}),
        "claude_recollect_calls": [
            {
                "run_id": item["run_id"],
                "claude_launches_cumulative": item["claude_launches_cumulative"],
                "model_calls": item["model_calls"],
            }
            for item in environment.get("claude_recollect", [])
        ],
    }


def write_results(summary: dict) -> None:
    RESULTS.mkdir(parents=True, exist_ok=True)
    TABLE.parent.mkdir(parents=True, exist_ok=True)
    summary_bytes = (
        json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    table_fields = [
        "condition",
        "n",
        "known_n",
        "rerun_count",
        "rerun_rate",
        "rerun_wilson_95",
        "status",
        "verdict",
    ]
    table_rows = []
    for item in summary["conditions"]:
        table_rows.append(
            {
                key: json.dumps(item[key], ensure_ascii=False)
                if isinstance(item[key], list)
                else item[key]
                for key in table_fields
            }
        )
    from io import StringIO

    buffer = StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=table_fields, lineterminator="\n")
    writer.writeheader()
    writer.writerows(table_rows)
    table_bytes = buffer.getvalue().encode("utf-8")
    if "--check" not in sys.argv:
        SUMMARY.write_bytes(summary_bytes)
        TABLE.write_bytes(table_bytes)
    elif (
        not SUMMARY.exists()
        or SUMMARY.read_bytes() != summary_bytes
        or not TABLE.exists()
        or TABLE.read_bytes() != table_bytes
    ):
        raise SystemExit("results가 raw와 다릅니다")


def main() -> None:
    if not rows():
        raise SystemExit("raw 데이터가 없습니다")
    write_results(analyze())


if __name__ == "__main__":
    main()
