"""비율 구간과 대응·군집 비교를 원정밀도로 계산한다."""

from __future__ import annotations

import math
import random
from collections import defaultdict

from protocol import SEED


def ratio(k: int, n: int) -> dict:
    if not n:
        return dict(k=k, n=n, value=None, ci=None)
    p, z = k / n, 1.959963984540054
    denominator = 1 + z * z / n
    center = (p + z * z / (2 * n)) / denominator
    width = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denominator
    return dict(k=k, n=n, value=p, ci=[max(0, center - width), min(1, center + width)])


def quantile(values: list[float], probability: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lo = math.floor(position)
    hi = math.ceil(position)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (position - lo)


def distribution(values: list[float]) -> dict:
    return dict(
        n=len(values),
        mean=sum(values) / len(values) if values else None,
        median=quantile(values, 0.5),
        p95=quantile(values, 0.95),
    )


def binomial_tail(k: int, n: int, p: float, *, upper: bool) -> float:
    indices = range(k, n + 1) if upper else range(k + 1)
    return (
        min(1.0, sum(math.comb(n, j) * p**j * (1 - p) ** (n - j) for j in indices))
        if n
        else 1.0
    )


def metric_counts(rows: list[dict]) -> dict:
    if not rows:
        return {}

    def metric(numerator: str, denominator: str | None = None) -> dict:
        selected = [r for r in rows if denominator is None or r[denominator]]
        return ratio(sum(bool(r[numerator]) for r in selected), len(selected))

    result = {
        "accuracy": metric("correct"),
        "format_error": metric("format_error"),
        "failure": metric("failure"),
    }
    if rows[0]["task"] == "continuation":
        result.update(
            recall=metric("pred_positive", "gold_positive"),
            precision=metric("gold_positive", "pred_positive"),
            false_join=metric("pred_positive", "gold_negative"),
        )
    else:
        result.update(
            request_accuracy=metric("request_correct"),
            request_precision=metric("gold_positive", "pred_positive"),
            request_recall=metric("pred_positive", "gold_positive"),
            target_accuracy=metric("target_correct", "gold_positive"),
            target_conditional=metric("target_correct", "true_positive"),
            kind_accuracy=metric("kind_correct", "gold_positive"),
            scope_semantic=metric("scope_semantic", "gold_exception"),
            scope_exact=metric("scope_exact", "gold_exception"),
            false_permanent=metric("false_permanent"),
            false_permanent_nonpermanent=metric("false_permanent", "gold_nonpermanent"),
            false_permanent_valid=metric("false_permanent", "valid"),
            false_permanent_precision_error=metric("false_permanent", "pred_permanent"),
        )
    return result


def cluster_intervals(rows: list[dict], names: list[str]) -> dict:
    groups = defaultdict(list)
    for row in rows:
        groups[row["cluster"]].append(row)
    keys = sorted(groups)
    rng = random.Random(SEED)
    values = {name: [] for name in names}
    if not keys:
        return {name: None for name in names}
    for _ in range(2000):
        sample = [r for key in rng.choices(keys, k=len(keys)) for r in groups[key]]
        metrics = metric_counts(sample)
        for name in names:
            value = metrics[name]["value"]
            if value is not None:
                values[name].append(value)
    return {
        name: dict(
            ci=[quantile(v, 0.025), quantile(v, 0.975)],
            valid_bootstraps=len(v),
            clusters=len(keys),
        )
        for name, v in values.items()
    }


def paired(left: list[dict], right: list[dict]) -> dict:
    a = {r["item_id"]: r for r in left}
    b = {r["item_id"]: r for r in right}
    ids = sorted(a.keys() & b.keys())
    better = sum(b[i]["correct"] and not a[i]["correct"] for i in ids)
    worse = sum(a[i]["correct"] and not b[i]["correct"] for i in ids)
    discordant = better + worse
    p = (
        min(1, 2 * binomial_tail(min(better, worse), discordant, 0.5, upper=False))
        if discordant
        else 1
    )
    diffs = [int(b[i]["correct"]) - int(a[i]["correct"]) for i in ids]
    rng = random.Random(SEED)
    samples = (
        [sum(rng.choices(diffs, k=len(diffs))) / len(diffs) for _ in range(2000)]
        if diffs
        else []
    )
    groups = defaultdict(list)
    for i, diff in zip(ids, diffs):
        groups[a[i]["cluster"]].append(diff)
    keys = sorted(groups)
    clusters = []
    for _ in range(2000):
        sample = (
            [d for key in rng.choices(keys, k=len(keys)) for d in groups[key]]
            if keys
            else []
        )
        if sample:
            clusters.append(sum(sample) / len(sample))
    return dict(
        n=len(ids),
        right_only=better,
        left_only=worse,
        difference=sum(diffs) / len(diffs) if diffs else None,
        p=p,
        ci=[quantile(samples, 0.025), quantile(samples, 0.975)],
        cluster_ci=[quantile(clusters, 0.025), quantile(clusters, 0.975)],
        clusters=len(keys),
    )


def holm(values: dict[str, float]) -> dict[str, float]:
    result = {}
    floor = 0.0
    for rank, (key, value) in enumerate(sorted(values.items(), key=lambda kv: kv[1])):
        floor = max(floor, min(1, (len(values) - rank) * value))
        result[key] = floor
    return result


def binomial_power(
    n: int, p0: float, p1: float, *, upper: bool, alpha: float = 0.05
) -> dict:
    critical = [
        k for k in range(n + 1) if n and binomial_tail(k, n, p0, upper=upper) <= alpha
    ]
    k = (min(critical) if upper else max(critical)) if critical else None
    return dict(
        n=n,
        p0=p0,
        p1=p1,
        alpha=alpha,
        critical=k,
        power=binomial_tail(k, n, p1, upper=upper) if k is not None else 0.0,
    )


def paired_power(n: int) -> dict:
    d, q = 0.4, 0.875
    value = 0.0
    for m in range(1, n + 1):
        for k in range(m + 1):
            if 2 * binomial_tail(min(k, m - k), m, 0.5, upper=False) < 0.05:
                value += (
                    math.comb(n, m)
                    * d**m
                    * (1 - d) ** (n - m)
                    * math.comb(m, k)
                    * q**k
                    * (1 - q) ** (m - k)
                )
    return dict(n=n, discordance=d, difference=0.3, power=value)


def power() -> dict:
    return dict(
        binomial=[
            binomial_power(n, p0, p1, upper=upper)
            for n, p0, p1, upper in [
                (70, 0.9, 0.99, True),
                (306, 0.95, 0.99, True),
                (450, 0.01, 0.0001, False),
            ]
        ],
        paired=paired_power(48),
    )
