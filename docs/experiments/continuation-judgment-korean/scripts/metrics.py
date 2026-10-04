"""비율 구간과 대응·군집 재표집용 지표를 계산한다."""

from __future__ import annotations

import math
import random
from collections import Counter, defaultdict
from statistics import NormalDist

THRESHOLDS = [round(0.50 + 0.05 * i, 2) for i in range(10)]


def ratio(k: int, n: int) -> dict:
    if not n:
        return {"k": k, "n": n, "value": None, "ci": None, "half_width": None}
    p, z = k / n, NormalDist().inv_cdf(0.975)
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return {
        "k": k,
        "n": n,
        "value": p,
        "ci": [max(0.0, center - half), min(1.0, center + half)],
        "half_width": half,
    }


def confusion(pairs: list[tuple[bool, bool]]) -> list[int]:
    return [
        sum(y and pred for y, pred in pairs),
        sum(not y and pred for y, pred in pairs),
        sum(y and not pred for y, pred in pairs),
        sum(not y and not pred for y, pred in pairs),
    ]


def measure(counts: list[int]) -> dict:
    tp, fp, fn, tn = counts
    return {
        "n": sum(counts),
        "tp": tp,
        "fp": fp,
        "fn": fn,
        "tn": tn,
        "precision": ratio(tp, tp + fp),
        "recall": ratio(tp, tp + fn),
        "false_join_rate": ratio(fp, fp + tn),
        "accuracy": ratio(tp + tn, sum(counts)),
        "f1": 2 * tp / (2 * tp + fp + fn) if 2 * tp + fp + fn else None,
    }


def values(counts: list[int]) -> dict:
    tp, fp, fn, tn = counts
    return {
        name: k / n if n else None
        for name, k, n in (
            ("precision", tp, tp + fp),
            ("recall", tp, tp + fn),
            ("false_join_rate", fp, fp + tn),
            ("accuracy", tp + tn, sum(counts)),
            ("f1", 2 * tp, 2 * tp + fp + fn),
        )
    }


def quantile(data: list[float], p: float) -> float | None:
    if not data:
        return None
    data = sorted(data)
    index = (len(data) - 1) * p
    lo = int(index)
    return data[lo] + (data[min(lo + 1, len(data) - 1)] - data[lo]) * (index - lo)


def interval(data: list[float]) -> list[float] | None:
    return [quantile(data, 0.025), quantile(data, 0.975)] if data else None


def mcnemar(b: int, c: int) -> float:
    n = b + c
    return (
        min(1.0, 2 * sum(math.comb(n, i) for i in range(min(b, c) + 1)) / 2**n)
        if n
        else 1.0
    )


def cluster_counts(cases: list[dict], seed: int) -> list[Counter]:
    groups = defaultdict(set)
    for case in cases:
        groups[case["project"]].add(case["session"])
    rng = random.Random(seed)
    output = []
    for _ in range(2000):
        selected = Counter()
        for project in sorted(groups):
            sessions = sorted(groups[project])
            selected.update(rng.choices(sessions, k=len(sessions)))
        output.append(selected)
    return output


def bootstrap(cases: list[dict], predictions: dict, draws: list[Counter]) -> dict:
    clusters = defaultdict(lambda: [0, 0, 0, 0])
    for case in cases:
        if case["id"] not in predictions:
            continue
        y, pred = case["label"] == "continue", predictions[case["id"]]
        index = 0 if y and pred else 1 if pred else 2 if y else 3
        clusters[case["session"]][index] += 1
    output = defaultdict(list)
    for draw in draws:
        counts = [sum(clusters[s][i] * n for s, n in draw.items()) for i in range(4)]
        for name, value in values(counts).items():
            if value is not None:
                output[name].append(value)
    return {
        name: {"ci": interval(data), "valid_draws": len(data)}
        for name, data in output.items()
    }


def bootstrap_delta(cases: list[dict], a: dict, b: dict, draws: list[Counter]) -> dict:
    clusters = defaultdict(lambda: [[0, 0, 0, 0], [0, 0, 0, 0]])
    for case in cases:
        if case["id"] not in a or case["id"] not in b:
            continue
        y = case["label"] == "continue"
        for condition, preds in enumerate((a, b)):
            pred = preds[case["id"]]
            index = 0 if y and pred else 1 if pred else 2 if y else 3
            clusters[case["session"]][condition][index] += 1
    output = defaultdict(list)
    for draw in draws:
        ab = [
            values(
                [
                    sum(clusters[s][condition][i] * n for s, n in draw.items())
                    for i in range(4)
                ]
            )
            for condition in (0, 1)
        ]
        for name in ab[0]:
            if ab[0][name] is not None and ab[1][name] is not None:
                output[name].append(ab[1][name] - ab[0][name])
    return {
        name: {"ci": interval(data), "valid_draws": len(data)}
        for name, data in output.items()
    }
