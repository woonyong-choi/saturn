"""패킷 인수 순서 추가 검증을 원본 로그에서 재계산한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
RAW = ROOT / ".local/experiments/same-session-compaction/handoff"
FORMAL = ROOT / ".local/experiments/same-session-compaction/formal"


def load_analysis() -> Any:
    spec = importlib.util.spec_from_file_location("analysis", HERE / "02-analyze.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("analysis unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate(row: dict[str, Any], source: dict[str, Any], analysis: Any) -> None:
    folder = RAW / f"scenario-{row['index']:03}"
    for name in ("packet_ready", "packet_answer"):
        events = analysis.read_events(folder / f"{name}.jsonl")
        result = analysis.result_of(events)
        if result["session_id"] != row["session_id"] or analysis.tool_calls(events):
            raise ValueError("handoff session or tool calls mismatch")
        if name == "packet_ready":
            if (result.get("result", "").strip() == "Ready") != row["ready"]:
                raise ValueError("ready score mismatch")
            continue
        text = result.get("result", "")
        if text != row["text"] or (source["answer"] in text) != row["exact"]:
            raise ValueError("handoff exact score mismatch")
        if result["total_cost_usd"] != row["cost_usd"]:
            raise ValueError("handoff cost mismatch")


def main() -> None:
    analysis = load_analysis()
    rows = analysis.read_events(RAW / "results.jsonl")
    if [r["index"] for r in rows] != list(range(1, 49)) or any(
        "error" in r for r in rows
    ):
        raise ValueError("handoff collection incomplete or failed")
    source = {r["index"]: r for r in analysis.read_events(FORMAL / "results.jsonl")}
    for row in rows:
        validate(row, source[row["index"]], analysis)
    pairs = [
        {
            "arms": {
                "packet": {"exact": r["exact"]},
                "full": source[r["index"]]["arms"]["full"],
            }
        }
        for r in rows
    ]
    hits = sum(r["exact"] for r in rows)
    summary = {
        "n": len(rows),
        "hits": hits,
        "rate": hits / len(rows),
        "wilson95": analysis.wilson(hits, len(rows)),
        "ready": sum(r["ready"] for r in rows),
        "errors": 0,
        "answer_tool_calls": 0,
        "paired_vs_full": analysis.paired(pairs, ("packet", "full"), 20261007),
        "claude_incremental_cost": analysis.numeric_summary(
            [r["cost_usd"] for r in rows]
        ),
        "elapsed_ms": analysis.numeric_summary([r["duration_ms"] for r in rows]),
        "position": {
            str(t): {
                "n": sum(r["target"] == t for r in rows),
                "hits": sum(r["exact"] for r in rows if r["target"] == t),
            }
            for t in range(1, 9)
        },
    }
    manifest = []
    for path in sorted(RAW.rglob("*")):
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"apikey_" in data:
            raise ValueError("secret-like value in handoff raw")
        manifest.append(f"{hashlib.sha256(data).hexdigest()}  {path.relative_to(RAW)}")
    manifest_text = "\n".join(manifest) + "\n"
    summary["manifest_sha256"] = hashlib.sha256(manifest_text.encode()).hexdigest()
    summary["raw_file_count"] = len(manifest)
    is_verify = "--verify" in sys.argv
    analysis.write_checked(
        HERE.parent / "data/HANDOFF-SHA256SUMS", manifest_text, is_verify=is_verify
    )
    analysis.write_checked(
        HERE.parent / "results/handoff-summary.json",
        json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        is_verify=is_verify,
    )
    table = [
        {
            "scenario": row["index"],
            "target": row["target"],
            "condition": "packet",
            "exact": int(row["exact"]),
            "ready": int(row["ready"]),
            "cost_usd": row["cost_usd"],
        }
        for row in rows
    ]
    analysis.write_table(
        HERE.parent / "results/tables/handoff.csv", table, is_verify=is_verify
    )
    print(
        json.dumps(
            {
                "n": len(rows),
                "hits": hits,
                "ready": summary["ready"],
                "verified": is_verify,
            }
        )
    )


if __name__ == "__main__":
    main()
