"""Publish aggregate development results without response bodies or task IDs."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def summarize(raw_path: Path, local_summary_path: Path) -> dict:
    rows = [
        json.loads(line) for line in raw_path.read_text(encoding="utf-8").splitlines()
    ]
    summary = json.loads(local_summary_path.read_text(encoding="utf-8"))
    ends = [row for row in rows if row["event"] == "end"]
    if (
        len(rows) != 120
        or len(ends) != 60
        or summary["replays_started"] != 60
        or summary["replays_finished"] != 60
    ):
        raise ValueError("development replay is incomplete")
    latencies = sorted(row["elapsed_ms"] for row in ends)
    statuses: dict[str, int] = {}
    models: dict[str, int] = {}
    usage_missing = 0
    for row in ends:
        status = str(row["status"])
        statuses[status] = statuses.get(status, 0) + 1
        reply = json.loads(row["received"]) if row["status"] == 200 else {}
        model = reply.get("model", "missing")
        models[model] = models.get(model, 0) + 1
        usage_missing += int("usage" not in reply)
    raw_sha = hashlib.sha256(raw_path.read_bytes()).hexdigest()
    lock_sha = hashlib.sha256(
        (ROOT / "eval" / "dev-source-lock.json").read_bytes()
    ).hexdigest()
    return {
        "phase": "development_smoke",
        "source_fixtures": summary["source_fixtures"],
        "replays": {
            "planned": summary["replays_expected"],
            "started": summary["replays_started"],
            "finished": summary["replays_finished"],
            "parse_ok": summary["parse_ok"],
        },
        "same_selection": summary["same_selection"],
        "required_wilson_lower": summary["threshold"],
        "meets_required_lower": summary["passes_90_percent_lower_bound"],
        "http_status": statuses,
        "response_model": models,
        "usage_missing": {"k": usage_missing, "n": len(ends)},
        "latency_ms": {
            "n": len(latencies),
            "sum": sum(latencies),
            "median": statistics.median(latencies),
            "p95_nearest_rank": latencies[math.ceil(0.95 * len(latencies)) - 1],
            "min": latencies[0],
            "max": latencies[-1],
        },
        "raw_sha256": raw_sha,
        "dev_source_lock_sha256": lock_sha,
        "confirmation": None,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--local-summary", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    rendered = (
        json.dumps(
            summarize(args.raw, args.local_summary),
            ensure_ascii=False,
            sort_keys=True,
            indent=2,
        )
        + "\n"
    )
    destination = ROOT / "results" / "summary.json"
    if args.check:
        if destination.read_text(encoding="utf-8") != rendered:
            raise ValueError("public summary differs from raw")
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
