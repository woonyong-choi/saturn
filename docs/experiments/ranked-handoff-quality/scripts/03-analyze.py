"""processed/의 채점 결과로 가설을 검정하고 results/에 쓴다.

사용: python3 scripts/03-analyze.py (실험 폴더에서 실행)
"""
from __future__ import annotations

import csv
import json
import math
import random
from collections import defaultdict
from pathlib import Path

SEED = 117
BOOTSTRAP_REPS = 2000
MARGIN = 0.05
Z95 = 1.959963984540054
EXPERIMENT = Path(__file__).resolve().parent.parent
PROCESSED = EXPERIMENT / "data" / "processed"
RESULTS = EXPERIMENT / "results"
CONDITIONS = ["judge-only", "rrf-only", "rrf-judge"]
COMPARISONS = {
    "H1": ("rrf-judge", "judge-only", "noninferiority"),
    "H2": ("rrf-judge", "rrf-only", "superiority"),
    "judge_vs_rrf": ("judge-only", "rrf-only", "exploratory"),
}


def read_csv(name: str) -> list[dict]:
    with (PROCESSED / f"{name}.csv").open(encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def norm_sf(z: float) -> float:
    return 0.5 * math.erfc(z / math.sqrt(2))


def wilson(k: int, n: int) -> list[float | None]:
    if n == 0:
        return [None, None]
    p = k / n
    centre = (p + Z95**2 / (2 * n)) / (1 + Z95**2 / n)
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n * n)) / (1 + Z95**2 / n)
    return [max(0.0, centre - half), min(1.0, centre + half)]


def binom_two_sided(b: int, c: int) -> float:
    m, low = b + c, min(b, c)
    tail = sum(math.comb(m, i) for i in range(low + 1)) / 2**m
    return min(1.0, 2 * tail)


def mcnemar(b: int, c: int) -> tuple[float, str]:
    if b + c == 0:
        return 1.0, "none"
    if b + c < 25:
        return binom_two_sided(b, c), "exact-binomial"
    chi = (b - c) ** 2 / (b + c)
    return math.erfc(math.sqrt(chi / 2)), "mcnemar"


def tango_noninferiority(b: int, c: int, n: int) -> float:
    """H0: 차이 ≤ -MARGIN에 대한 단측 Tango 점수 검정 p값."""
    d0 = -MARGIN
    a_coef = 2 * n
    b_coef = -b - c + (2 * n - b + c) * d0
    c_coef = -c * d0 * (1 - d0)
    p21 = (math.sqrt(b_coef**2 - 4 * a_coef * c_coef) - b_coef) / (2 * a_coef)
    z = (b - c - n * d0) / math.sqrt(n * (2 * p21 + d0 * (1 - d0)))
    return norm_sf(z)


def newcombe(a: int, b: int, c: int, d: int) -> list[float]:
    n = a + b + c + d
    p1, p2 = (a + b) / n, (a + c) / n
    l1, u1 = wilson(a + b, n)
    l2, u2 = wilson(a + c, n)
    denom = math.sqrt((a + b) * (c + d) * (a + c) * (b + d))
    phi = (a * d - b * c) / denom if denom else 0.0
    diff = p1 - p2
    lower = diff - math.sqrt(max((p1 - l1) ** 2 - 2 * phi * (p1 - l1) * (u2 - p2) + (u2 - p2) ** 2, 0))
    upper = diff + math.sqrt(max((u1 - p1) ** 2 - 2 * phi * (u1 - p1) * (p2 - l2) + (p2 - l2) ** 2, 0))
    return [lower, upper]


def paired(scores: dict, new: str, old: str, units: list[str]) -> dict:
    a = b = c = d = 0
    for unit in units:
        x, y = scores[unit][new], scores[unit][old]
        a += x and y
        b += x and not y
        c += y and not x
        d += not x and not y
    n = a + b + c + d
    return {"n": n, "a": a, "b": b, "c": c, "d": d,
            "diff": (b - c) / n if n else None, "ci95": newcombe(a, b, c, d) if n else [None, None]}


def bootstrap(scores: dict, new: str, old: str, units: list[str]) -> list[float]:
    clusters = defaultdict(list)
    for unit in units:
        clusters[unit.split("-")[0]].append(unit)
    keys = sorted(clusters)
    rng = random.Random(SEED)
    diffs = []
    for _ in range(BOOTSTRAP_REPS):
        picked = [u for _ in keys for u in clusters[rng.choice(keys)]]
        diffs.append(sum(scores[u][new] - scores[u][old] for u in picked) / len(picked))
    diffs.sort()
    return [diffs[int(0.025 * BOOTSTRAP_REPS)], diffs[int(0.975 * BOOTSTRAP_REPS) - 1]]


def holm(pvalues: dict) -> dict:
    ordered = sorted(pvalues, key=pvalues.get)
    adjusted, running = {}, 0.0
    for rank, key in enumerate(ordered):
        running = max(running, min(1.0, (len(ordered) - rank) * pvalues[key]))
        adjusted[key] = running
    return adjusted


def verdict(name: str, adjusted_p: float, ci: list[float]) -> str:
    bound = -MARGIN if name == "H1" else 0.0
    if adjusted_p <= 0.05 and ci[0] > bound:
        return "채택"
    if ci[1] <= bound:
        return "기각"
    return "보류"


def accuracy_table(scores: dict, units: list[str]) -> dict:
    table = {}
    for cond in CONDITIONS:
        k = sum(scores[u][cond] for u in units)
        table[cond] = {"k": k, "n": len(units), "rate": k / len(units) if units else None,
                       "ci95": wilson(k, len(units))}
    return table


def main() -> None:
    sessions = read_csv("sessions")
    trials = read_csv("trials")
    flow = defaultdict(int)
    bad_units = set()
    for s in sessions:
        flow[s["status"]] += 1
        if s["status"] != "ok":
            bad_units.add((s["scenario_id"], s["provider"]))
    scores, meta = defaultdict(dict), {}
    for t in trials:
        if (t["scenario_id"], t["provider"]) in bad_units:
            continue
        scores[t["unit"]][t["condition"]] = int(t["correct"])
        meta[t["unit"]] = t
    units = sorted(u for u in scores if all(c in scores[u] for c in CONDITIONS))

    comparisons = {name: paired(scores, new, old, units) for name, (new, old, _) in COMPARISONS.items()}
    h1, h2 = comparisons["H1"], comparisons["H2"]
    h1["p"] = tango_noninferiority(h1["b"], h1["c"], h1["n"]) if h1["n"] else 1.0
    h1["test"] = "tango-noninferiority"
    h2["p"], h2["test"] = mcnemar(h2["b"], h2["c"])
    adjusted = holm({"H1": h1["p"], "H2": h2["p"]})
    for name in ("H1", "H2"):
        comparisons[name]["p_holm"] = adjusted[name]
        comparisons[name]["verdict"] = verdict(name, adjusted[name], comparisons[name]["ci95"])
        new, old, _ = COMPARISONS[name]
        comparisons[name]["cluster_bootstrap_ci95"] = bootstrap(scores, new, old, units) if units else None
    explore = comparisons["judge_vs_rrf"]
    explore["p"], explore["test"] = mcnemar(explore["b"], explore["c"])

    by_group = {}
    for field in ("qtype", "provider"):
        for value in sorted({meta[u][field] for u in units}):
            subset = [u for u in units if meta[u][field] == value]
            by_group[f"{field}={value}"] = {
                "accuracy": accuracy_table(scores, subset),
                "diff_h1": paired(scores, "rrf-judge", "judge-only", subset),
                "diff_h2": paired(scores, "rrf-judge", "rrf-only", subset),
            }

    evidence = read_csv("evidence")
    missed = [e for e in evidence if e["in_top_n"] == "0"]
    semantic = sum(int(e["semantic"]) for e in missed)
    relation_mix = defaultdict(int)
    for e in evidence:
        relation_mix[e["relation"]] += 1

    cost = {}
    for cond in CONDITIONS:
        rows = [s for s in sessions if s["condition"] == cond]
        cost[cond] = {key: sum(float(s[key] or 0) for s in rows) / len(rows) if rows else None
                      for key in ("packet_tokens", "judge_calls", "judge_failures")}

    summary = {
        "flow": {"sessions": len(sessions), **dict(flow), "excluded_units": len(bad_units),
                 "analyzed_trials_per_condition": len(units)},
        "accuracy": accuracy_table(scores, units),
        "confirmatory": {k: comparisons[k] for k in ("H1", "H2")},
        "exploratory": {
            "judge_only_vs_rrf_only": explore,
            "by_group": by_group,
            "rrf_missed": {"k": semantic, "n": len(missed),
                           "rate": semantic / len(missed) if missed else None,
                           "ci95": wilson(semantic, len(missed)),
                           "evidence_relation_mix": dict(sorted(relation_mix.items()))},
            "mean_cost_per_session": cost,
        },
    }
    (RESULTS / "tables").mkdir(parents=True, exist_ok=True)
    (RESULTS / "figures").mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                                          encoding="utf-8")
    with (RESULTS / "tables" / "accuracy.csv").open("w", encoding="utf-8", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["group", "condition", "k", "n", "rate", "ci95_low", "ci95_high"])
        groups = {"all": summary["accuracy"], **{g: v["accuracy"] for g, v in by_group.items()}}
        for group, table in groups.items():
            for cond, row in table.items():
                writer.writerow([group, cond, row["k"], row["n"], row["rate"], *row["ci95"]])
    if units:
        write_accuracy_chart(summary["accuracy"])


def write_accuracy_chart(accuracy: dict) -> None:
    """조건마다 정답률과 95% Wilson 신뢰구간을 막대로 쓴다."""
    n = accuracy[CONDITIONS[0]]["n"]
    header = f"""chart bar
title "조건별 정답률"
subtitle "n={n}. 오차 막대는 95% Wilson 신뢰구간"
x "정답률(%)"
decimals 1

series accuracy "정답률" role=main
"""
    rows = [{"label": condition, "accuracy": percent(row["rate"]), "accuracy.low": percent(row["ci95"][0]),
             "accuracy.high": percent(row["ci95"][1])} for condition, row in accuracy.items()]
    figures = RESULTS / "figures"
    (figures / "accuracy.muto").write_text(f'{header}data "accuracy.json"\n', encoding="utf-8")
    (figures / "accuracy.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def percent(value: float) -> float:
    """비율을 백분율로 바꾼다. 이진 부동소수점 잔차는 버린다."""
    return round(value * 100, 8)


if __name__ == "__main__":
    main()
