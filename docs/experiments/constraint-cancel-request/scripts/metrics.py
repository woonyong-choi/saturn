"""이항 구간, 대응 차이와 군집 불확실성을 계산한다."""

from __future__ import annotations

import math
import random
from collections import defaultdict

from common import SEED


def ratio(k: int, n: int) -> dict:
    if not n:
        return dict(k=k, n=n, value=None, low=None, high=None)
    p = k / n
    z = 1.959963984540054
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return dict(k=k, n=n, value=p, low=max(0, c - h), high=min(1, c + h))


def tail(k: int, n: int, p: float, *, upper: bool = True) -> float:
    if not n:
        return 1.0
    ks = range(k, n + 1) if upper else range(k + 1)
    return min(1.0, sum(math.comb(n, j) * p**j * (1 - p) ** (n - j) for j in ks))


def quantile(xs: list[float], p: float) -> float | None:
    if not xs:
        return None
    xs = sorted(xs)
    at = (len(xs) - 1) * p
    a = int(at)
    b = min(a + 1, len(xs) - 1)
    return xs[a] + (xs[b] - xs[a]) * (at - a)


def decision(r: dict, threshold: float) -> dict:
    if not r["valid"]:
        return dict(request=False, target="none", actions=[], ask=False, correct=False)
    target = max(r["candidates"], key=r["candidates"].get)
    top = r["candidates"][target]
    request = r["request_p"] >= threshold
    if r["form"] == "pair":
        gate = r["constraint_p"] < 0.7 and r["request_p"] >= 0.5
        actions = [k for k, v in r["candidates"].items() if gate and v >= threshold]
        ask = gate and not actions and 0.5 <= top < threshold
    else:
        winner = target if top > r["none_p"] else "none"
        actions = [winner] if winner != "none" and top >= threshold else []
        ask = winner != "none" and 0.5 <= top < threshold
    correct = (
        (request and target == r["target"]) if r["intent"] != "none" else not request
    )
    return dict(
        request=request, target=target, actions=actions, ask=ask, correct=correct
    )


def counts(r: dict, threshold: float) -> dict:
    d = decision(r, threshold)
    positive = r["intent"] != "none"
    tp = int(positive and d["request"])
    pred = int(d["request"])
    hit = int(tp and d["target"] == r["target"])
    target_set = [r["target"]] if r["intent"] == "release" else []
    return dict(
        precision=(tp, pred),
        recall=(tp, int(positive)),
        target_accuracy=(hit, tp),
        target_reach=(hit, int(positive)),
        false_release=(int(not positive and bool(d["actions"])), int(not positive)),
        ask_rate=(int(d["ask"]), 1),
        accuracy=(int(d["correct"]), 1),
        action_accuracy=(int(r["valid"] and d["actions"] == target_set), 1),
        limited_release=(
            int(r["intent"] in ("partial", "conditional") and bool(d["actions"])),
            int(r["intent"] in ("partial", "conditional")),
        ),
    )


def aggregate(rows: list[dict], threshold: float) -> dict:
    sums = {
        key: [0, 0]
        for key in (
            "precision",
            "recall",
            "target_accuracy",
            "target_reach",
            "false_release",
            "ask_rate",
            "accuracy",
            "action_accuracy",
            "limited_release",
        )
    }
    for r in rows:
        for key, pair in counts(r, threshold).items():
            for j in (0, 1):
                sums[key][j] += pair[j]
    return {key: ratio(*pair) for key, pair in sums.items()}


def bootstrap(rows: list[dict], metric: str, threshold: float) -> dict:
    groups = defaultdict(lambda: [0, 0])
    for r in rows:
        k, n = counts(r, threshold)[metric]
        groups[r["cluster"]][0] += k
        groups[r["cluster"]][1] += n
    groups = list(groups.values())
    rng = random.Random(SEED)
    values = []
    empty = 0
    for _ in range(2000):
        picked = rng.choices(groups, k=len(groups))
        k = sum(x[0] for x in picked)
        n = sum(x[1] for x in picked)
        if n:
            values.append(k / n)
        else:
            empty += 1
    return dict(
        low=quantile(values, 0.025),
        high=quantile(values, 0.975),
        empty=empty,
        clusters=len(groups),
    )


def paired(left: list[dict], right: list[dict], threshold: float) -> dict:
    a = {r["item_id"]: r for r in left}
    b = {r["item_id"]: r for r in right}
    ids = sorted(a.keys() & b.keys())
    deltas = []
    byte_deltas = []
    groups = defaultdict(list)
    first_only = 0
    second_only = 0
    for key in ids:
        av = decision(a[key], threshold)["correct"]
        bv = decision(b[key], threshold)["correct"]
        delta = int(bv) - int(av)
        deltas.append(delta)
        groups[a[key]["cluster"]].append(delta)
        first_only += av and not bv
        second_only += bv and not av
        byte_deltas.append(b[key]["request_bytes"] - a[key]["request_bytes"])
    if not ids:
        return dict(
            n=0,
            first_only=0,
            second_only=0,
            difference=None,
            low=None,
            high=None,
            cluster_low=None,
            cluster_high=None,
            p=1.0,
            byte_difference_median=None,
            byte_low=None,
            byte_high=None,
        )
    rng = random.Random(SEED)
    means = []
    cluster_means = []
    medians = []
    values = list(groups.values())
    for _ in range(2000):
        sample = rng.choices(deltas, k=len(deltas))
        means.append(sum(sample) / len(sample))
        sample = [x for g in rng.choices(values, k=len(values)) for x in g]
        cluster_means.append(sum(sample) / len(sample))
        medians.append(quantile(rng.choices(byte_deltas, k=len(byte_deltas)), 0.5))
    return dict(
        n=len(ids),
        first_only=first_only,
        second_only=second_only,
        difference=sum(deltas) / len(deltas),
        low=quantile(means, 0.025),
        high=quantile(means, 0.975),
        cluster_low=quantile(cluster_means, 0.025),
        cluster_high=quantile(cluster_means, 0.975),
        p=tail(second_only, first_only + second_only, 0.5),
        byte_difference_median=quantile(byte_deltas, 0.5),
        byte_low=quantile(medians, 0.025),
        byte_high=quantile(medians, 0.975),
    )


def holm(ps: dict) -> dict:
    ordered = sorted(ps, key=ps.get)
    out = {}
    last = 0.0
    for index, key in enumerate(ordered):
        last = max(last, min(1.0, ps[key] * (len(ps) - index)))
        out[key] = last
    return out
