"""표준 라이브러리만 쓰는 구간과 검정."""
from __future__ import annotations

import math
import random


def wilson(k: int, n: int, z: float = 1.959964) -> list[float] | None:
    if n == 0:
        return None
    p = k / n
    d = 1 + z * z / n
    center = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return [round(max(0.0, center - half), 4), round(min(1.0, center + half), 4)]


def binom_cdf(k: int, n: int, p: float) -> float:
    return sum(math.comb(n, i) * p**i * (1 - p) ** (n - i) for i in range(k + 1))


def cp_upper(k: int, n: int, alpha: float = 0.05) -> float | None:
    """Clopper-Pearson 단측 (1 - alpha) 상한."""
    if n == 0:
        return None
    if k >= n:
        return 1.0
    lo, hi = k / n, 1.0
    for _ in range(80):
        mid = (lo + hi) / 2
        if binom_cdf(k, n, mid) > alpha:
            lo = mid
        else:
            hi = mid
    return round(hi, 4)


def mcnemar_one_sided(loss: int, gain: int) -> float:
    """손실 쌍이 이득 쌍보다 많은지: P(X >= loss), X ~ Bin(loss + gain, 0.5)."""
    n = loss + gain
    if n == 0:
        return 1.0
    return round(sum(math.comb(n, i) for i in range(loss, n + 1)) / 2**n, 6)


def percentile(values: list[float], q: float) -> float | None:
    if not values:
        return None
    s = sorted(values)
    pos = (len(s) - 1) * q
    lo, hi = math.floor(pos), math.ceil(pos)
    return round(s[lo] + (s[hi] - s[lo]) * (pos - lo), 6)


def bootstrap_mean(values: list[float], seed: int = 542, draws: int = 10000) -> dict | None:
    if not values:
        return None
    rng = random.Random(seed)
    n = len(values)
    means = sorted(sum(values[rng.randrange(n)] for _ in range(n)) / n for _ in range(draws))
    return dict(n=n, mean=round(sum(values) / n, 6), lo=round(means[int(0.025 * draws)], 6), hi=round(means[int(0.975 * draws) - 1], 6))
