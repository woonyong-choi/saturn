"""사후 진단은 사전 채점을 유지하며 형식 실패와 총호출 비용을 분리한다."""

from __future__ import annotations

import importlib.util
import csv
import json
import sys
from collections import Counter
from pathlib import Path
from statistics import median

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from runtime import PRIVATE, PUBLIC, read, response_text, write

spec = importlib.util.spec_from_file_location(
    "scoring", PUBLIC / "scripts/02-analyze.py"
)
SCORING = importlib.util.module_from_spec(spec)
spec.loader.exec_module(SCORING)


def embedded_value(text: str) -> dict | None:
    values = []
    decoder = json.JSONDecoder()
    for i, ch in enumerate(text):
        if ch != "{":
            continue
        try:
            value, _ = decoder.raw_decode(text[i:])
        except ValueError:
            continue
        if isinstance(value, dict) and value not in values:
            values.append(value)
    return values[0] if len(values) == 1 else None


def composed(trials: list[str]) -> dict:
    rows = [SCORING.result(t) for t in trials]
    costs = [r["cost_usd"] for r in rows]
    times = [r["latency_s"] for r in rows]
    usage = Counter()
    for r in rows:
        usage.update(
            {k: v for k, v in r["usage"].items() if isinstance(v, (int, float))}
        )
    return {
        "trials": trials,
        "latency_s": sum(times) if all(t is not None for t in times) else None,
        "cost_usd": sum(costs) if all(c is not None for c in costs) else None,
        "known_cost_usd": sum(c for c in costs if c is not None),
        "unknown_cost_calls": sum(c is None for c in costs),
        "reported_usage": dict(usage),
    }


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    strict = read(PUBLIC / "results/summary.json")
    diagnostics = []
    for case in cases["routing"]:
        for model in strict["routing"]["models"]:
            trial = "route-run-" + case["id"] + "-" + model
            raw = read(PRIVATE / "raw" / f"{trial}.json")
            text, _ = response_text(raw)
            r = SCORING.result(trial)
            value = embedded_value(text) if r["value"] is None else r["value"]
            success = (
                SCORING.expression_passes(value, case["tests"])
                if "tests" in case
                else value == case["expected"]
            )
            diagnostics.append(
                {
                    "trial_id": trial,
                    "model": model,
                    "strict_json": r["value"] is not None,
                    "content_success": success,
                }
            )
    policies = {}
    for lane in ("constraints", "context"):
        for policy in strict[lane]["conditions"]:
            cells = []
            for case in cases[lane]:
                if lane == "constraints":
                    prefix = "constraint"
                    trials = ["constraint-extract-" + case["id"]]
                    if policy == "llm":
                        trials += ["constraint-reference-" + case["id"]]
                else:
                    prefix = "context"
                    trials = []
                if policy == "jev":
                    trials += [prefix + "-jev-" + case["id"]]
                trials += [prefix + "-run-" + case["id"] + "-" + policy]
                cells.append(composed(trials))
            policies[lane + "-" + policy] = {
                "cells": cells,
                "composed_latency_median_s": median(c["latency_s"] for c in cells),
                "known_cost_usd": sum(c["known_cost_usd"] for c in cells),
                "unknown_cost_calls": sum(c["unknown_cost_calls"] for c in cells),
            }
    raw = [read(p) for p in sorted((PRIVATE / "raw").glob("*.json"))]
    output = {
        "analysis": "post-hoc diagnostic; strict preregistered scores unchanged",
        "format_failures": sum(not r["strict_json"] for r in diagnostics),
        "diagnostic_models": {
            m: SCORING.rate(
                [r["content_success"] for r in diagnostics if r["model"] == m]
            )
            for m in strict["routing"]["models"]
        },
        "diagnostic_rows": diagnostics,
        "composed_policies": policies,
        "calls": dict(Counter(r["kind"] for r in raw)),
        "status": dict(Counter(r["status"] for r in raw)),
        "direct_fallback_count": sum(
            d["direct_fallback"] for d in strict["routing"]["decisions"]
        ),
        "context_body_reduction": 0.75,
        "protocol_commit": read(PRIVATE / "collection-seal.json")["commit"],
    }
    write(PUBLIC / "results/diagnostics.json", output)
    strict["diagnostics"] = {
        k: output[k]
        for k in (
            "analysis",
            "format_failures",
            "calls",
            "status",
            "direct_fallback_count",
            "diagnostic_models",
            "context_body_reduction",
            "protocol_commit",
        )
    }
    strict["diagnostics"]["composed_policies"] = {
        name: {k: v for k, v in values.items() if k != "cells"}
        for name, values in policies.items()
    }
    write(PUBLIC / "results/summary.json", strict)
    observations = []
    for row in read(PRIVATE / "processed.json"):
        original = read(PRIVATE / "raw" / (row["trial_id"] + ".json"))
        observations.append(
            dict(
                row,
                run_id=output["protocol_commit"][:7],
                input_id=row["case_id"],
                ts_utc=original["ts_utc"],
            )
        )
    (PRIVATE / "observations.jsonl").write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in observations)
    )
    table = PUBLIC / "results" / "outcomes.csv"
    fields = [
        "run_id",
        "trial_id",
        "input_id",
        "condition",
        "lane",
        "ts_utc",
        "status",
        "success",
        "latency_s",
        "cost_usd",
    ]
    with table.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(observations)
    print(
        json.dumps(
            {
                k: output[k]
                for k in ("format_failures", "calls", "status", "direct_fallback_count")
            }
        )
    )


if __name__ == "__main__":
    main()
