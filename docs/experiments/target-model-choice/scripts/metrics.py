"""비율 구간과 프로젝트 군집을 보존한 대응 비교를 계산한다."""

from __future__ import annotations

import math
import random
from collections import defaultdict
from statistics import median

from runtime import SEED


def percentile(values: list, fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def interval(k: int, n: int) -> list[float] | None:
    if not n:
        return None
    z = 1.959963984540054
    p = k / n
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return [max(0, center - half), min(1, center + half)]


def rate(values: list) -> dict:
    return dict(
        k=sum(values),
        n=len(values),
        rate=sum(values) / len(values) if values else None,
        wilson95=interval(sum(values), len(values)),
    )


# cost: time O(r*g), heap O(r+g); vars: r = 10000, g = projects; basis: estimate
def cluster_interval(rows: list, field: str) -> list[float] | None:
    groups = defaultdict(list)
    for row in rows:
        if row.get(field) is not None:
            groups[row["project_id"]].append(row[field])
    totals = [(sum(group), len(group)) for _, group in sorted(groups.items())]
    if not totals:
        return None
    rng = random.Random(SEED)
    estimates = []
    for _ in range(10000):
        selected = rng.choices(totals, k=len(totals))
        estimates.append(sum(k for k, n in selected) / sum(n for k, n in selected))
    return [percentile(estimates, 0.025), percentile(estimates, 0.975)]


def measured_rate(rows: list, field: str) -> dict:
    result = rate([row[field] for row in rows if row.get(field) is not None])
    result["cluster95"] = cluster_interval(rows, field)
    return result


def paired(rows: list) -> dict:
    values = [
        dict(row, delta=int(row["effective_hit"]) - int(row["manual_hit"]))
        for row in rows
    ]
    b = sum(r["effective_hit"] and not r["manual_hit"] for r in rows)
    c = sum(r["manual_hit"] and not r["effective_hit"] for r in rows)
    p = (
        min(
            1, 2 * sum(math.comb(b + c, i) for i in range(min(b, c) + 1)) / 2 ** (b + c)
        )
        if b + c
        else 1.0
    )
    return dict(
        delta=sum(r["delta"] for r in values) / len(values) if values else None,
        cluster95=cluster_interval(values, "delta"),
        auto_only=b,
        manual_only=c,
        mcnemar_exact_p=p,
        n=len(values),
    )


def timing(values: list) -> dict:
    return dict(
        n=len(values),
        median_s=median(values) if values else None,
        p90_s=percentile(values, 0.9),
        min_s=min(values) if values else None,
        max_s=max(values) if values else None,
    )
