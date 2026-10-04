"""반복을 독립 표본으로 세지 않는 비율과 검정력을 계산한다."""

from __future__ import annotations

import math
import random

from storage import SEED


def ratio(k: int, n: int) -> dict:
    if not n:
        return {"k": k, "n": n, "value": None, "ci95": [None, None], "half_width": None}
    p = k / n
    z2 = 1.96**2
    denominator = 1 + z2 / n
    center = (p + z2 / (2 * n)) / denominator
    half_width = 1.96 * math.sqrt(p * (1 - p) / n + z2 / (4 * n**2)) / denominator
    return {
        "k": k,
        "n": n,
        "value": p,
        "ci95": [max(0, center - half_width), min(1, center + half_width)],
        "half_width": half_width,
    }


def binomial_tail(k: int, n: int, p: float) -> float:
    if not n:
        return 1.0
    return min(
        1.0,
        sum(
            math.exp(
                math.lgamma(n + 1)
                - math.lgamma(i + 1)
                - math.lgamma(n - i + 1)
                + i * math.log(p)
                + (n - i) * math.log1p(-p)
            )
            for i in range(k, n + 1)
        ),
    )


def power_requirement() -> dict:
    for n in range(1, 1000):
        critical = next(
            (k for k in range(n + 1) if binomial_tail(k, n, 0.8) <= 0.025), n + 1
        )
        power = binomial_tail(critical, n, 0.9)
        if power >= 0.8:
            return {
                "n": n,
                "critical_successes": critical,
                "power": power,
                "null": 0.8,
                "alternative": 0.9,
                "alpha": 0.025,
                "target": 0.8,
            }
    raise RuntimeError("power search exhausted")


def metrics(rows: list[dict], threshold: float) -> dict:
    tp = fp = fn = tn = 0
    for row in rows:
        predicted = row["probability"] is not None and row["probability"] >= threshold
        gold = row["gold"]
        tp += predicted and gold
        fp += predicted and not gold
        fn += not predicted and gold
        tn += not predicted and not gold
    return {
        "threshold": threshold,
        "n": len(rows),
        "tp": tp,
        "fp": fp,
        "fn": fn,
        "tn": tn,
        "precision": ratio(tp, tp + fp),
        "recall": ratio(tp, tp + fn),
        "accuracy": ratio(tp + tn, len(rows)),
        "f1": 2 * tp / (2 * tp + fp + fn) if 2 * tp + fp + fn else None,
        "ask_rate": ratio(
            sum(
                row["probability"] is not None and 0.5 <= row["probability"] < threshold
                for row in rows
            ),
            len(rows),
        ),
    }


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    values = sorted(values)
    index = fraction * (len(values) - 1)
    low, high = math.floor(index), math.ceil(index)
    return values[low] + (values[high] - values[low]) * (index - low)


def cluster_bootstrap(rows: list[dict], threshold: float) -> dict:
    groups = {}
    for row in rows:
        groups.setdefault(row["conversation_id"], []).append(row)
    if len(groups) < 2:
        return {
            "clusters": len(groups),
            "precision_ci95": [None, None],
            "recall_ci95": [None, None],
        }
    randomizer = random.Random(SEED)
    values = {"precision": [], "recall": []}
    clusters = list(groups.values())
    for _ in range(2000):
        selected = [
            row
            for group in randomizer.choices(clusters, k=len(clusters))
            for row in group
        ]
        result = metrics(selected, threshold)
        for key in values:
            if result[key]["value"] is not None:
                values[key].append(result[key]["value"])
    return {
        "clusters": len(groups),
        **{
            key + "_ci95": [percentile(samples, 0.025), percentile(samples, 0.975)]
            for key, samples in values.items()
        },
    }
