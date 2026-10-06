#!/usr/bin/env python3
"""조건별 성공과 첫 패킷 차이를 같은 계열에서 비교한다."""

from __future__ import annotations

import csv
import json
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]


def analyze() -> dict:
    with (BASE / "data" / "processed" / "trials.csv").open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    by = {(row["case"], row["arm"]): row for row in rows}
    cases = sorted({row["case"] for row in rows})
    changes = 0
    jev_only = 0
    rank_only = 0
    arm_totals = {}
    for arm in ("off", "rank", "jev"):
        selected = [by[(case, arm)] for case in cases]
        times = sorted(float(row["latency_s"]) for row in selected if row["latency_s"])
        arm_totals[arm] = dict(trials=len(selected), correct=sum(int(row["correct"]) for row in selected), missing=sum(row["status"] == "missing" for row in selected), related_applied=sum(int(row["related_applied"]) for row in selected), lookups=sum(int(row["lookups"]) for row in selected if row["lookups"]), provider_tokens=sum(int(row["provider_tokens"]) for row in selected if row["provider_tokens"]), router_tokens=sum(int(row["router_tokens"]) for row in selected if row["router_tokens"]), mean_latency_s=round(sum(times) / len(times), 3) if times else None, median_latency_s=round((times[(len(times) - 1) // 2] + times[len(times) // 2]) / 2, 3) if times else None)
    for case in cases:
        rank, jev = by[(case, "rank")], by[(case, "jev")]
        if rank["first_packet_hash"] and jev["first_packet_hash"] and rank["first_packet_hash"] != jev["first_packet_hash"]:
            changes += 1
        jev_only += int(jev["correct"] == "1" and rank["correct"] == "0")
        rank_only += int(rank["correct"] == "1" and jev["correct"] == "0")
    return dict(cases=len(cases), trials=len(rows), packet_different=changes, packet_different_by_kind={kind: sum(bool(by[(case, "rank")]["first_packet_hash"] and by[(case, "rank")]["first_packet_hash"] != by[(case, "jev")]["first_packet_hash"]) for case in cases if case.startswith(kind)) for kind in ("exact", "stale", "synonym", "middle")}, jev_only=jev_only, rank_only=rank_only, h1="진행" if len(cases) < 12 else ("충족" if changes >= 3 else "미충족"), h2="탐색" if changes < 3 else ("신호" if jev_only > rank_only and arm_totals["jev"]["correct"] > arm_totals["rank"]["correct"] else "신호 없음"), arms=arm_totals)


def main() -> None:
    summary = analyze()
    results = BASE / "results"
    results.mkdir(exist_ok=True)
    (results / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
