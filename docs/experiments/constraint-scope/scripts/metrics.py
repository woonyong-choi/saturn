"""빈 분모와 대응·프로젝트 상관을 보존해 비율과 차이를 계산한다."""

from __future__ import annotations

import itertools
import math
import random
from collections import defaultdict
from statistics import NormalDist

from runtime import SEED

REPEATS = 5000


def interval(values: list[float], alpha: float = 0.05) -> list[float] | None:
    if not values:
        return None
    values = sorted(values)
    return [values[int((len(values) - 1) * q)] for q in (alpha / 2, 1 - alpha / 2)]


def quantile(values: list[float], q: float) -> float | None:
    if not values:
        return None
    return sorted(values)[max(0, math.ceil(len(values) * q) - 1)]


def rate(k: int, n: int, alpha: float = 0.05) -> dict:
    if n == 0:
        return dict(k=k, n=n, value=None, ci=None)
    z = NormalDist().inv_cdf(1 - alpha / 2)
    p = k / n
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return dict(k=k, n=n, value=p, ci=[max(0, center - half), min(1, center + half)])


def is_positive(row: dict, threshold: float) -> bool:
    return row["valid"] and row["probability"] >= threshold


def evaluate(
    data: list[dict], threshold: float, lower: float | None = None, alpha: float = 0.05
) -> dict:
    known = [r for r in data if r["gold"] != "uncertain"]
    positive = sum(r["gold"] == "constraint" for r in known)
    auto = [r for r in known if is_positive(r, threshold)]
    tp = sum(r["gold"] == "constraint" for r in auto)
    correct = sum(
        r["valid"] and (is_positive(r, threshold) == (r["gold"] == "constraint"))
        for r in known
    )
    result = dict(
        precision=rate(tp, len(auto), alpha),
        recall=rate(tp, positive, alpha),
        accuracy=rate(correct, len(known), alpha),
        failed=sum(not r["valid"] for r in data),
        unresolved_auto=sum(
            r["gold"] == "uncertain" and is_positive(r, threshold) for r in data
        ),
    )
    if lower is not None:
        asks = [r for r in data if r["valid"] and lower <= r["probability"] < threshold]
        combined = sum(
            r["gold"] == "constraint" and is_positive(r, lower) for r in known
        )
        result.update(
            ask_rate=rate(len(asks), len(data), alpha),
            combined_recall=rate(combined, positive, alpha),
            ask_positive=rate(
                sum(r["gold"] == "constraint" for r in asks),
                sum(r["gold"] != "uncertain" for r in asks),
            ),
        )
    return result


def precision(data: list[dict], threshold: float) -> float | None:
    auto = [r for r in data if r["gold"] != "uncertain" and is_positive(r, threshold)]
    return sum(r["gold"] == "constraint" for r in auto) / len(auto) if auto else None


def pairs(first: list[dict], second: list[dict]) -> list[tuple]:
    right = {r["sample_id"]: r for r in second}
    return [(r, right[r["sample_id"]]) for r in first]


def difference(data: list[tuple], threshold: float) -> float | None:
    a, b = (
        precision([p[0] for p in data], threshold),
        precision([p[1] for p in data], threshold),
    )
    return b - a if a is not None and b is not None else None


# cost: time O(b*n), heap O(b+n); vars: b = 5000, n = inputs; basis: estimate
def compare(first: list[dict], second: list[dict], threshold: float) -> dict:
    data = pairs(first, second)
    groups = defaultdict(list)
    for pair in data:
        groups[pair[0]["project_id"]].append(pair)
    rng = random.Random(SEED)
    keys = sorted(groups)
    cluster, individual = [], []
    for _ in range(REPEATS):
        sample = [p for k in rng.choices(keys, k=len(keys)) for p in groups[k]]
        d = difference(sample, threshold)
        if d is not None:
            cluster.append(d)
        d = difference(rng.choices(data, k=len(data)), threshold)
        if d is not None:
            individual.append(d)
    observed = difference(data, threshold)
    permuted = []
    for switches in itertools.product((False, True), repeat=len(keys)):
        sample = [
            (b, a) if switch else (a, b)
            for key, switch in zip(keys, switches)
            for a, b in groups[key]
        ]
        d = difference(sample, threshold)
        if d is not None:
            permuted.append(d)
    return dict(
        value=observed,
        ci=interval(cluster, 0.00625),
        ci95=interval(cluster),
        individual_ci=interval(individual, 0.00625),
        undefined_cluster=REPEATS - len(cluster),
        bootstrap_repeats=REPEATS,
        permutation_p=sum(d >= observed - 1e-12 for d in permuted) / len(permuted)
        if permuted and observed is not None
        else None,
        permutations=len(permuted),
        projects=len(keys),
        n=len(data),
        first_only=sum(
            is_positive(a, threshold) and not is_positive(b, threshold) for a, b in data
        ),
        second_only=sum(
            is_positive(b, threshold) and not is_positive(a, threshold) for a, b in data
        ),
    )


def judge_difference(comparisons: list[dict]) -> str:
    if any(
        r["value"] is None or r["undefined_cluster"] / REPEATS > 0.05
        for r in comparisons
    ):
        return "보류"
    if all(r["ci"][0] >= 0 and r["value"] >= 0.05 for r in comparisons):
        return "채택"
    if any(r["ci"][1] < 0.05 for r in comparisons):
        return "기각"
    return "보류"


# cost: time O(b*n), heap O(n+b); vars: b = 5000, n = inputs; basis: estimate
def cluster_metrics(data: list[dict], threshold: float) -> dict:
    groups = defaultdict(list)
    for row in data:
        groups[row["project_id"]].append(row)
    rng = random.Random(SEED)
    keys = sorted(groups)
    values = defaultdict(list)
    for _ in range(REPEATS):
        sample = [r for k in rng.choices(keys, k=len(keys)) for r in groups[k]]
        metrics = evaluate(sample, threshold)
        for key in ("precision", "recall", "accuracy"):
            if metrics[key]["value"] is not None:
                values[key].append(metrics[key]["value"])
    return {key: interval(values[key]) for key in ("precision", "recall", "accuracy")}


# cost: time O(b*n log n), heap O(n+b); vars: b = 5000, n = observations; basis: estimate
def performance(base: list[dict], treatment: list[dict]) -> dict:
    groups = defaultdict(lambda: [[], []])
    for index, data in enumerate((base, treatment)):
        for row in data:
            groups[row["sample_id"]][index].append(row)
    keys = sorted(groups)
    rng = random.Random(SEED)
    latencies, ratios = [], []
    for _ in range(REPEATS):
        sample = [groups[k] for k in rng.choices(keys, k=len(keys))]
        times = [
            r["latency_s"]
            for pair in sample
            for r in pair[1]
            if r["latency_s"] is not None
        ]
        costs = [
            [
                r["cost_usd"]
                for pair in sample
                for r in pair[i]
                if r["cost_usd"] is not None
            ]
            for i in (0, 1)
        ]
        if times:
            latencies.append(quantile(times, 0.95))
        if all(costs) and sum(costs[0]):
            ratios.append(
                (sum(costs[1]) / len(costs[1])) / (sum(costs[0]) / len(costs[0]))
            )
    observed_times = [r["latency_s"] for r in treatment if r["latency_s"] is not None]
    costs = [
        [r["cost_usd"] for r in data if r["cost_usd"] is not None]
        for data in (base, treatment)
    ]
    ratio = (
        (sum(costs[1]) / len(costs[1])) / (sum(costs[0]) / len(costs[0]))
        if all(costs) and sum(costs[0])
        else None
    )
    return dict(
        p95_s=quantile(observed_times, 0.95),
        median_s=quantile(observed_times, 0.5),
        p95_ci=interval(latencies, 0.003125),
        cost_ratio=ratio,
        cost_ratio_ci=interval(ratios, 0.003125),
        mean_cost_usd=sum(costs[1]) / len(costs[1]) if costs[1] else None,
        n=len(treatment),
        cost_missing=len(treatment) - len(costs[1]),
    )
