#!/usr/bin/env python3
"""개발 표본의 오접합·재현율·묻기·판단 토큰을 집계한다."""

from __future__ import annotations

import csv
import json
import math
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[2]


def wilson(k: int, n: int) -> list[float] | None:
    if n == 0:
        return None
    z = 1.959964
    p = k / n
    d = 1 + z * z / n
    center = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return [round(max(0.0, center - half), 4), round(min(1.0, center + half), 4)]


def cluster_interval(rows: list[dict]) -> dict | None:
    pairs = [row for row in rows if row["old_tokens"] and row["new_tokens"]]
    if not pairs:
        return None
    groups = defaultdict(list)
    for row in pairs:
        groups[row["session"]].append(int(row["new_tokens"]) - int(row["old_tokens"]))
    names = sorted(groups)
    rng = random.Random(544202)
    estimates = []
    for _ in range(3000):
        values = [difference for _ in names for difference in groups[names[rng.randrange(len(names))]]]
        estimates.append(sum(values) / len(values))
    estimates.sort()
    observed = [difference for values in groups.values() for difference in values]
    return dict(pairs=len(observed), sessions=len(names), mean=round(sum(observed) / len(observed), 2), cluster95=[round(estimates[75], 2), round(estimates[2924], 2)])


def analyze(run_id: str) -> dict:
    run = ROOT / ".runtime" / "continuation-atomic-judgment" / run_id
    with (run / "processed.csv").open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    new = [row for row in rows if row["label"] == "new"]
    continuing = [row for row in rows if row["label"] == "continue"]
    false_join = sum(row["new_action"] == "continue" for row in new)
    recall = sum(row["new_action"] == "continue" for row in continuing)
    ask = sum(row["new_action"] == "ask" for row in rows)
    old_false_join = sum(row["old_action"] == "continue" for row in new)
    old_recall = sum(row["old_action"] == "continue" for row in continuing)
    interval = cluster_interval(rows)
    h1 = bool(new) and false_join / len(new) <= 0.05 and wilson(false_join, len(new))[1] <= 0.10
    h2 = bool(continuing) and wilson(recall, len(continuing))[0] >= 0.60
    h3 = bool(rows) and ask / len(rows) <= 0.40 and interval is not None and interval["cluster95"][1] < 0
    return dict(run_id=run_id, total=len(rows), labels=dict(Counter(row["label"] for row in rows)), statuses=dict(Counter(row["status"] for row in rows)), new=dict(n=len(new), false_join=false_join, interval95=wilson(false_join, len(new)), old_false_join=old_false_join), continue_label=dict(n=len(continuing), recalled=recall, interval95=wilson(recall, len(continuing)), old_recalled=old_recall), ask=dict(n=len(rows), count=ask, interval95=wilson(ask, len(rows))), token_difference=interval, h1=h1, h2=h2, h3=h3, development_gate="통과" if h1 and h2 and h3 else "미통과")


def main() -> None:
    summary = analyze(sys.argv[1])
    results = BASE / "results"
    results.mkdir(exist_ok=True)
    (results / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
