"""실패를 정답으로 세지 않고 대응·프로젝트 상관을 보존한 구간을 계산한다."""

from __future__ import annotations

import math
import random
from collections import defaultdict

from runtime import SEED


def wilson(k: int, n: int, z: float = 1.959963984540054) -> list[float] | None:
    if not n:
        return None
    p = k / n
    denominator = 1 + z * z / n
    center = (p + z * z / (2 * n)) / denominator
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denominator
    return [max(0, center - half), min(1, center + half)]


def rate(k: int, n: int) -> dict:
    return dict(k=k, n=n, value=k / n if n else None, ci95=wilson(k, n))


def counts(data: list[dict], threshold: float) -> dict:
    tp = fp = tn = fn = failed = 0
    for row in data:
        positive = row["gold"] == "constraint"
        prediction = row["probability"] is not None and row["probability"] >= threshold
        if not row["valid"]:
            failed += 1
            fn += int(positive)
        elif prediction:
            tp += int(positive)
            fp += int(not positive)
        else:
            tn += int(not positive)
            fn += int(positive)
    return dict(
        tp=tp,
        fp=fp,
        tn=tn,
        fn=fn,
        failed=failed,
        n=len(data),
        positives=sum(r["gold"] == "constraint" for r in data),
    )


def values(data: list[dict], threshold: float) -> dict:
    c = counts(data, threshold)
    tp, fp, tn, fn = (c[k] for k in ("tp", "fp", "tn", "fn"))
    return dict(
        accuracy=(tp + tn) / c["n"] if c["n"] else None,
        precision=tp / (tp + fp) if tp + fp else None,
        recall=tp / c["positives"] if c["positives"] else None,
        f1=2 * tp / (2 * tp + fp + fn) if 2 * tp + fp + fn else None,
    )


def interval(values_: list[float]) -> list[float] | None:
    if not values_:
        return None
    ordered = sorted(values_)
    return [ordered[int((len(ordered) - 1) * q)] for q in (0.025, 0.975)]


# cost: time O(b*n), heap O(n+b); vars: b = 2000 resamples, n = rows; basis: estimate
def bootstrap(data: list[dict], threshold: float, *, clustered: bool) -> dict:
    rng = random.Random(SEED)
    projects = defaultdict(list)
    for row in data:
        projects[row["project_id"]].append(row)
    keys = sorted(projects)
    samples = defaultdict(list)
    if not data:
        return {name: None for name in ("accuracy", "precision", "recall", "f1")}
    for _ in range(2000):
        resampled = (
            [r for key in rng.choices(keys, k=len(keys)) for r in projects[key]]
            if clustered
            else rng.choices(data, k=len(data))
        )
        for name, value in values(resampled, threshold).items():
            if value is not None:
                samples[name].append(value)
    return {
        name: interval(samples[name])
        for name in ("accuracy", "precision", "recall", "f1")
    }


def metrics(data: list[dict], threshold: float = 0.5) -> dict:
    c = counts(data, threshold)
    result = dict(
        counts=c,
        accuracy=rate(c["tp"] + c["tn"], c["n"]),
        precision=rate(c["tp"], c["tp"] + c["fp"]),
        recall=rate(c["tp"], c["positives"]),
    )
    result["f1"] = dict(
        value=values(data, threshold)["f1"],
        ci95=bootstrap(data, threshold, clustered=False)["f1"],
    )
    result["cluster_ci95"] = bootstrap(data, threshold, clustered=True)
    result["conditional_accuracy"] = rate(c["tp"] + c["tn"], c["n"] - c["failed"])
    return result


def paired_difference(first: list[dict], second: list[dict]) -> dict:
    left = {r["sample_id"]: r for r in first}
    right = {r["sample_id"]: r for r in second}
    differences = []
    for sample_id in sorted(left.keys() & right.keys()):
        correctness = []
        for row in (left[sample_id], right[sample_id]):
            correctness.append(
                int(
                    row["valid"]
                    and ((row["probability"] >= 0.5) == (row["gold"] == "constraint"))
                )
            )
        differences.append(correctness[1] - correctness[0])
    if not differences:
        return dict(n=0, value=None, ci95=None)
    rng = random.Random(SEED)
    distribution = [
        sum(rng.choices(differences, k=len(differences))) / len(differences)
        for _ in range(2000)
    ]
    return dict(
        n=len(differences),
        value=sum(differences) / len(differences),
        ci95=interval(distribution),
        first_only=differences.count(-1),
        second_only=differences.count(1),
    )
