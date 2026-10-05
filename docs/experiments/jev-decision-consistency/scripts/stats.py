"""비율 구간과 군집 bootstrap. 같은 입력이면 같은 값을 낸다."""
import math
import random

Z95 = 1.959963984540054
BOOTSTRAP_RUNS = 2000
BOOTSTRAP_SEED = 5445


def wilson(k: int, n: int) -> tuple[float | None, float | None]:
    if n == 0:
        return None, None
    p = k / n
    denominator = 1 + Z95**2 / n
    center = (p + Z95**2 / (2 * n)) / denominator
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n**2)) / denominator
    return max(0.0, center - half), min(1.0, center + half)


def cluster_bootstrap(flags: list[tuple[str, int]]) -> tuple[float | None, float | None]:
    """(군집, 0/1) 목록에서 군집을 복원 추출해 비율의 percentile 95% 구간을 낸다."""
    if not flags:
        return None, None
    clusters: dict[str, list[int]] = {}
    for cluster, flag in flags:
        clusters.setdefault(cluster, []).append(flag)
    names = sorted(clusters)
    rng = random.Random(BOOTSTRAP_SEED)
    rates = []
    for _ in range(BOOTSTRAP_RUNS):
        hit = total = 0
        for _ in names:
            values = clusters[names[rng.randrange(len(names))]]
            hit += sum(values)
            total += len(values)
        rates.append(hit / total)
    rates.sort()
    return rates[int(0.025 * BOOTSTRAP_RUNS)], rates[int(0.975 * BOOTSTRAP_RUNS) - 1]


def rnd(value: float | None, digits: int = 6) -> float | None:
    return None if value is None else round(value, digits)
