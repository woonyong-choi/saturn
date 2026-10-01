"""비율과 대응 차이의 신뢰구간. 표준 라이브러리만 쓴다."""
import math
import random

Z = 1.959964


def wilson(k, n):
    if n == 0:
        return (float("nan"), float("nan"))
    p = k / n
    denom = 1 + Z * Z / n
    center = (p + Z * Z / (2 * n)) / denom
    half = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / denom
    return (max(0.0, center - half), min(1.0, center + half))


def paired_table(pairs):
    """pairs는 (첫째 값, 둘째 값) 0/1 목록. a=둘 다 1, b=첫째만, c=둘째만, d=둘 다 0."""
    a = sum(1 for x, y in pairs if x and y)
    b = sum(1 for x, y in pairs if x and not y)
    c = sum(1 for x, y in pairs if not x and y)
    d = sum(1 for x, y in pairs if not x and not y)
    return a, b, c, d


def newcombe_paired(pairs):
    """첫째 - 둘째의 대응 차이와 Newcombe(1998) 방법 10의 95% 신뢰구간."""
    a, b, c, d = paired_table(pairs)
    n = a + b + c + d
    if n == 0:
        return {"n": 0, "diff": float("nan"), "lower": float("nan"), "upper": float("nan"), "a": 0, "b": 0, "c": 0, "d": 0}
    p1, p2 = (a + b) / n, (a + c) / n
    l1, u1 = wilson(a + b, n)
    l2, u2 = wilson(a + c, n)
    big_a = (a + b) * (c + d) * (a + c) * (b + d)
    big_b = a * d - b * c
    if big_b > 0:
        big_c = big_b - n / 2
    elif big_b >= -n / 2:
        big_c = 0
    else:
        big_c = big_b + n / 2
    phi = big_c / math.sqrt(big_a) if big_a > 0 else 0.0
    diff = p1 - p2
    lower = diff - math.sqrt(max(0.0, (p1 - l1) ** 2 + (u2 - p2) ** 2 - 2 * phi * (p1 - l1) * (u2 - p2)))
    upper = diff + math.sqrt(max(0.0, (u1 - p1) ** 2 + (p2 - l2) ** 2 - 2 * phi * (u1 - p1) * (p2 - l2)))
    return {"n": n, "diff": diff, "lower": lower, "upper": upper, "a": a, "b": b, "c": c, "d": d}


def cluster_bootstrap(groups, reps, seed):
    """groups는 {작업: [(첫째, 둘째), ...]}. 작업을 복원추출해 차이의 95% 백분위 구간을 낸다."""
    rng = random.Random(seed)
    keys = sorted(groups)
    diffs = []
    for _ in range(reps):
        total = n = 0
        for key in (rng.choice(keys) for _ in keys):
            for x, y in groups[key]:
                total += x - y
                n += 1
        diffs.append(total / n if n else 0.0)
    diffs.sort()
    return diffs[int(0.025 * reps)], diffs[int(0.975 * reps) - 1]


def verdict(lower, upper, margin):
    """차이(첫째 - 둘째)의 구간이 -margin보다 완전히 위면 채택, 완전히 아래면 기각, 아니면 보류."""
    if lower > -margin:
        return "채택"
    if upper <= -margin:
        return "기각"
    return "보류"
