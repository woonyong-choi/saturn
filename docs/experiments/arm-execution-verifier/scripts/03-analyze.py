"""실패·제외·재시도·실제 적용률을 모두 집계한다. 같은 입력이면 같은 바이트를 낸다."""

from __future__ import annotations

import json
from statistics import median

import contract
from arm_runtime import CALLER, PRIVATE, RESULTS, analysis, read, rows


def summarize(items: list[dict]) -> dict:
    merged = [contract.merge_usage(t["usage"]) for t in items]
    latencies = [t["latency_s"] for t in items if t["latency_s"] is not None]
    selectors = [t for t in items if t["arm"] in ("llm", "jev")]
    reasons = sorted({t["selection"]["reason"] for t in selectors if t["selection"]["fallback"]})
    return {
        "trials": len(items),
        "success": analysis.rate([t["check"]["success"] for t in items]),
        "statuses": {s: sum(t["status"] == s for t in items) for s in contract.STATUSES},
        "selector_applied": analysis.rate([not t["selection"]["fallback"] for t in selectors])
        if selectors
        else None,
        "fallback_reasons": {
            r: sum(t["selection"]["reason"] == r for t in selectors) for r in reasons
        },
        "usage_known_sum": {
            k: sum(m["known"][k] for m in merged) for k in contract.USAGE_FIELDS
        },
        "usage_unreported_calls": sum(m["unreported_calls"] for m in merged),
        "usage_calls": sum(m["calls"] for m in merged),
        "latency_median_s": median(latencies) if latencies else None,
        "latency_sum_s": round(sum(latencies), 6),
    }


def paired(items: list[dict], base: list[dict]) -> dict:
    wrap = lambda ts: [dict(case_id=t["task_id"], success=t["check"]["success"]) for t in ts]
    return analysis.paired(wrap(items), wrap(base))


def main() -> None:
    trials = [json.loads(s) for s in (PRIVATE / "trials.jsonl").read_text().splitlines()]
    cases = read(PRIVATE / "cases.json")
    ledger = rows(PRIVATE / "calls.jsonl")
    out: dict = {
        "caller": CALLER,
        "evidence": "pipeline-only: stub caller, not a measurement of any selector"
        if CALLER == "stub"
        else "live calls through official CLI and TypeSafe HTTPS",
        "sample": {
            "tasks": len(cases),
            "families": len({c["family_id"] for c in cases}),
            "excluded_fixtures": 0,
        },
        "retries": len(ledger) - len({r["trial_id"] for r in ledger}),
        "splits": {},
    }
    for split in ("sealed", "dev"):
        part = [t for t in trials if t["split"] == split]
        arms = {a: [t for t in part if t["arm"] == a] for a in contract.ARMS}
        out["splits"][split] = {
            "arms": {a: summarize(ts) for a, ts in arms.items() if ts},
            "paired_vs_full": {a: paired(ts, arms["full"]) for a, ts in arms.items() if ts and a != "full"},
            "paired_vs_code": {a: paired(ts, arms["code"]) for a, ts in arms.items() if ts and a not in ("full", "code")},
        }
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(
        json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    )
    print("analyzed", len(trials), "trials")


if __name__ == "__main__":
    main()
