"""프로세스 유지 조건의 원응답과 축약 경계를 검증해 대응 차이를 계산한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
RAW = ROOT / ".local/experiments/same-session-compaction/persistent"
FORMAL = ROOT / ".local/experiments/same-session-compaction/formal"


def load_analysis() -> Any:
    spec = importlib.util.spec_from_file_location("analysis", HERE / "02-analyze.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("analysis unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate(
    row: dict[str, Any], source: dict[str, Any], analysis: Any
) -> dict[str, Any]:
    folder = RAW / f"scenario-{row['index']:03}"
    streams = {
        f"{arm}_{turn}": analysis.read_events(folder / f"{arm}_{turn}.jsonl")
        for arm in ("fast", "native")
        for turn in ("compact", "answer")
    }
    parent_events = analysis.read_events(FORMAL / folder.name / "base_4.jsonl")
    parent = [event for event in parent_events if event.get("type") == "assistant"][-1][
        "uuid"
    ]
    audit = analysis.verify_compaction(row, streams, parent)
    audit["costs"] = {}
    audit["input_tokens"] = {}
    baseline = source["costs"]["full_answer"]
    fast_compact = analysis.result_of(streams["fast_compact"])
    if audit["hook_applied"] and abs(fast_compact["total_cost_usd"] - baseline) > 1e-8:
        raise ValueError(
            "persistent fork cost baseline is not the last source cumulative value"
        )
    for arm in ("fast", "native"):
        events = streams[f"{arm}_answer"]
        result = analysis.result_of(events)
        if result["session_id"] != row["arms"][arm]["session_id"]:
            raise ValueError("persistent session changed")
        if analysis.tool_calls(events) or analysis.tool_calls(
            streams[f"{arm}_compact"]
        ):
            raise ValueError("unexpected persistent tool call")
        text = result.get("result", "")
        if (
            text != row["arms"][arm]["text"]
            or (source["answer"] in text) != row["arms"][arm]["exact"]
        ):
            raise ValueError("persistent score mismatch")
        if row["arms"][arm]["base_parent"] != parent:
            raise ValueError("recorded resume parent mismatch")
        audit["costs"][arm] = result["total_cost_usd"] - baseline
        usage = result["usage"]
        audit["input_tokens"][arm] = sum(
            usage.get(key, 0)
            for key in (
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            )
        )
    return audit


def main() -> None:
    analysis = load_analysis()
    rows = analysis.read_events(RAW / "results.jsonl")
    if [r["index"] for r in rows] != list(range(1, 49)) or any(
        "error" in r for r in rows
    ):
        raise ValueError("persistent collection incomplete or failed")
    source = {r["index"]: r for r in analysis.read_events(FORMAL / "results.jsonl")}
    audits = [validate(row, source[row["index"]], analysis) for row in rows]
    summary = {
        "n": len(rows),
        "conditions": {},
        "errors": 0,
        "answer_tool_calls": 0,
        "hook_applied": sum(a["hook_applied"] for a in audits),
        "fast_dropped_calls": sum(a["dropped"] for a in audits),
        "paired": analysis.paired(rows, ("fast", "native"), 20261009),
        "claude_incremental_cost": {},
        "input_tokens": {},
        "position": {},
        "cost_baseline": "last source cumulative full_answer; verified by zero-cost fast compaction",
    }
    for arm in ("fast", "native"):
        hits = sum(r["arms"][arm]["exact"] for r in rows)
        summary["conditions"][arm] = {
            "hits": hits,
            "n": len(rows),
            "rate": hits / len(rows),
            "wilson95": analysis.wilson(hits, len(rows)),
        }
        summary["claude_incremental_cost"][arm] = analysis.numeric_summary(
            [a["costs"][arm] for a in audits]
        )
        summary["input_tokens"][arm] = analysis.numeric_summary(
            [a["input_tokens"][arm] for a in audits]
        )
    for target in range(1, 9):
        selected = [row for row in rows if row["target"] == target]
        summary["position"][str(target)] = {
            "n": len(selected),
            **{
                arm: sum(row["arms"][arm]["exact"] for row in selected)
                for arm in ("fast", "native")
            },
        }
    applied = [row for row, audit in zip(rows, audits) if audit["hook_applied"]]
    summary["paired_hook_only"] = analysis.paired(applied, ("fast", "native"), 20261009)
    summary["hook_fallbacks"] = len(rows) - len(applied)
    manifest = []
    for path in sorted(RAW.rglob("*")):
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"apikey_" in data:
            raise ValueError("secret-like value in persistent raw")
        manifest.append(f"{hashlib.sha256(data).hexdigest()}  {path.relative_to(RAW)}")
    manifest_text = "\n".join(manifest) + "\n"
    summary["manifest_sha256"] = hashlib.sha256(manifest_text.encode()).hexdigest()
    summary["raw_file_count"] = len(manifest)
    is_verify = "--verify" in sys.argv
    analysis.write_checked(
        HERE.parent / "data/PERSISTENT-SHA256SUMS", manifest_text, is_verify=is_verify
    )
    analysis.write_checked(
        HERE.parent / "results/persistent-summary.json",
        json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        is_verify=is_verify,
    )
    table = [
        {
            "scenario": row["index"],
            "target": row["target"],
            "condition": arm,
            "exact": int(row["arms"][arm]["exact"]),
            "input_tokens": audit["input_tokens"][arm],
            "cost_usd": round(audit["costs"][arm], 8),
        }
        for row, audit in zip(rows, audits)
        for arm in ("fast", "native")
    ]
    analysis.write_table(
        HERE.parent / "results/tables/persistent.csv", table, is_verify=is_verify
    )
    print(
        json.dumps(
            {
                "n": len(rows),
                "hits": {
                    a: summary["conditions"][a]["hits"] for a in ("fast", "native")
                },
                "hook_applied": summary["hook_applied"],
                "verified": is_verify,
            }
        )
    )


if __name__ == "__main__":
    main()
