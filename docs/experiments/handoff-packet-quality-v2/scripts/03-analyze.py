"""processed/의 채점 결과로 가설을 판정하고 results/에 쓴다.

사용: python3 scripts/03-analyze.py (실험 폴더에서 실행)
"""
from __future__ import annotations

import csv
import json
import math
import random
from collections import defaultdict
from pathlib import Path

SEED = 218
BOOTSTRAP_REPS = 10000
MARGIN = 0.10
ALPHA = 0.05
Z95 = 1.959963984540054
EXPERIMENT = Path(__file__).resolve().parent.parent
PROCESSED = EXPERIMENT / "data" / "processed"
RESULTS = EXPERIMENT / "results"
CONDITIONS = ["judge-all", "rrf-fallback", "no-packet"]
COMPARISONS = {"H1": ("judge-all", "no-packet"), "H2": ("judge-all", "rrf-fallback")}


def read_csv(name: str) -> list[dict]:
    with (PROCESSED / f"{name}.csv").open(encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def wilson(k: int, n: int) -> list:
    if n == 0:
        return [None, None]
    p = k / n
    centre = (p + Z95**2 / (2 * n)) / (1 + Z95**2 / n)
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n * n)) / (1 + Z95**2 / n)
    return [max(0.0, centre - half), min(1.0, centre + half)]


def mcnemar(b: int, c: int) -> tuple:
    """b+c < 25면 정확 이항 검정, 아니면 McNemar 검정(연속성 보정 없음)."""
    if b + c == 0:
        return 1.0, "none"
    if b + c < 25:
        low = min(b, c)
        tail = sum(math.comb(b + c, i) for i in range(low + 1)) / 2 ** (b + c)
        return min(1.0, 2 * tail), "exact-binomial"
    return math.erfc(math.sqrt((b - c) ** 2 / (b + c) / 2)), "mcnemar"


def newcombe(a: int, b: int, c: int, d: int) -> list:
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


def paired(scores: dict, new: str, old: str, units: list) -> dict:
    a = b = c = d = 0
    for unit in units:
        x, y = scores[unit][new], scores[unit][old]
        a += x and y
        b += x and not y
        c += y and not x
        d += not x and not y
    n = a + b + c + d
    p, test = mcnemar(b, c)
    return {"n": n, "a": a, "b": b, "c": c, "d": d, "diff": (b - c) / n if n else None,
            "newcombe_ci95": newcombe(a, b, c, d) if n else [None, None], "mcnemar_p": p, "mcnemar_test": test}


def bootstrap(scores: dict, new: str, old: str, units: list) -> dict:
    """시나리오 단위 군집 부트스트랩. 95% 백분위 구간과 기준 차이 MARGIN에 대한 양측 p값."""
    clusters = defaultdict(lambda: [0, 0])
    for unit in units:
        entry = clusters[unit.split("-")[0]]
        entry[0] += scores[unit][new] - scores[unit][old]
        entry[1] += 1
    sums = [clusters[key] for key in sorted(clusters)]
    rng = random.Random(SEED)
    diffs = []
    for _ in range(BOOTSTRAP_REPS):
        picked = [sums[rng.randrange(len(sums))] for _ in sums]
        diffs.append(sum(x[0] for x in picked) / sum(x[1] for x in picked))
    diffs.sort()
    below = sum(1 for d in diffs if d <= MARGIN)
    above = sum(1 for d in diffs if d >= MARGIN)
    p = min(1.0, 2 * (min(below, above) + 1) / (BOOTSTRAP_REPS + 1))
    return {"clusters": len(sums), "ci95": [diffs[int(0.025 * BOOTSTRAP_REPS)], diffs[int(0.975 * BOOTSTRAP_REPS) - 1]],
            "p_margin": p}


def holm(pvalues: dict) -> dict:
    ordered = sorted(pvalues, key=pvalues.get)
    adjusted, running = {}, 0.0
    for rank, key in enumerate(ordered):
        running = max(running, min(1.0, (len(ordered) - rank) * pvalues[key]))
        adjusted[key] = running
    return adjusted


def verdict(name: str, p_holm: float, ci: list) -> tuple:
    """H1: 하한 ≥ +10%p면 채택, 상한 < +10%p면 기각. H2: 상한 ≤ 10%p면 채택, 하한 > 10%p면 기각."""
    low, high = ci
    if name == "H1":
        if low >= MARGIN and p_holm <= ALPHA:
            return "채택", "패킷 설계 유지"
        if high < MARGIN and p_holm <= ALPHA:
            return "기각", "패킷 구성 재설계 design 이슈"
    else:
        if high <= MARGIN and p_holm <= ALPHA:
            return "채택", "대체 규칙 유지"
        if low > MARGIN and p_holm <= ALPHA:
            return "기각", "judge 실패 시 재시도와 지연 규칙 design 이슈"
    return "보류", "시나리오를 늘려 새 실행 id로 다시 수집"


def accuracy_table(scores: dict, units: list) -> dict:
    table = {}
    for cond in CONDITIONS:
        k = sum(scores[u][cond] for u in units)
        table[cond] = {"k": k, "n": len(units), "rate": k / len(units) if units else None,
                       "ci95": wilson(k, len(units))}
    return table


def mean(values: list) -> float | None:
    return sum(values) / len(values) if values else None


def main() -> None:
    sessions = read_csv("sessions")
    trials = read_csv("trials")
    flow = defaultdict(int)
    bad_units, reasons = set(), defaultdict(int)
    for s in sessions:
        flow[s["status"]] += 1
        if s["status"] != "ok":
            if (s["scenario_id"], s["provider"]) not in bad_units:
                reasons[s["status"]] += 1
            bad_units.add((s["scenario_id"], s["provider"]))
    scores, meta = defaultdict(dict), {}
    for t in trials:
        if (t["scenario_id"], t["provider"]) in bad_units or t["correct"] == "":
            continue
        scores[t["unit"]][t["condition"]] = int(t["correct"])
        meta[t["unit"]] = t
    units = sorted(u for u in scores if all(c in scores[u] for c in CONDITIONS))

    summary_accuracy = accuracy_table(scores, units)
    comparisons = {}
    for name, (new, old) in COMPARISONS.items():
        comparisons[name] = {"new": new, "old": old, **paired(scores, new, old, units),
                             **{"bootstrap": bootstrap(scores, new, old, units)}}
    adjusted = holm({n: comparisons[n]["bootstrap"]["p_margin"] for n in comparisons})
    for name, comp in comparisons.items():
        comp["p_holm"] = adjusted[name]
        comp["verdict"], comp["action"] = verdict(name, adjusted[name], comp["bootstrap"]["ci95"])

    by_group = {}
    for field in ("qtype", "provider"):
        for value in sorted({meta[u][field] for u in units}):
            subset = [u for u in units if meta[u][field] == value]
            by_group[f"{field}={value}"] = {
                "accuracy": accuracy_table(scores, subset),
                **{f"diff_{n}": paired(scores, new, old, subset) for n, (new, old) in COMPARISONS.items()},
            }

    by_subtype = {}
    for sub in ("sum", "codes", "first", "date"):
        subset = [u for u in units if meta[u]["qsub"] == sub]
        by_subtype[sub] = {"qtype": meta[subset[0]]["qtype"], "accuracy": accuracy_table(scores, subset)}

    evidence = read_csv("evidence")
    evidence_by_qtype = {}
    for cond in ("judge-all", "rrf-fallback"):
        for qtype in sorted({e["qtype"] for e in evidence}):
            sub = [e for e in evidence if e["condition"] == cond and e["qtype"] == qtype]
            kk = sum(int(e["in_packet"]) for e in sub)
            evidence_by_qtype[f"{cond},qtype={qtype}"] = {"k": kk, "n": len(sub), "rate": kk / len(sub), "ci95": wilson(kk, len(sub))}
    coverage, relation_mix = {}, defaultdict(int)
    for cond in ("judge-all", "rrf-fallback"):
        rows = [e for e in evidence if e["condition"] == cond]
        k = sum(int(e["in_packet"]) for e in rows)
        coverage[cond] = {"k": k, "n": len(rows), "rate": k / len(rows) if rows else None, "ci95": wilson(k, len(rows))}
        by_relation = {}
        for relation in sorted({e["relation"] for e in rows}):
            sub = [e for e in rows if e["relation"] == relation]
            kk = sum(int(e["in_packet"]) for e in sub)
            by_relation[relation] = {"k": kk, "n": len(sub), "rate": kk / len(sub), "ci95": wilson(kk, len(sub))}
        coverage[cond]["by_relation"] = by_relation
    for e in evidence:
        if e["condition"] == "judge-all":
            relation_mix[e["relation"]] += 1

    packets = read_csv("packets")
    cost = {}
    for cond in ("judge-all", "rrf-fallback"):
        rows = [p for p in packets if p["condition"] == cond]
        cost[cond] = {key: mean([float(p[key]) for p in rows]) for key in
                      ("packet_tokens", "included_items", "judge_calls", "judge_failures", "judged")}
        cost[cond]["scenarios"] = len(rows)
    cost["judge-all"]["scenarios_with_failure"] = sum(1 for p in packets if p["condition"] == "judge-all"
                                                    and int(p["judge_failures"]) > 0)
    cost["no-packet"] = {"packet_tokens": mean([float(s["packet_tokens"]) for s in sessions
                                                if s["condition"] == "no-packet"])}

    previous = json.loads((EXPERIMENT.parent / "handoff-packet-quality" / "results" / "summary.json")
                          .read_text(encoding="utf-8"))
    vs_previous = {"overall": {c: {"previous": previous["accuracy"][c]["rate"], "now": summary_accuracy[c]["rate"],
                                   "delta": summary_accuracy[c]["rate"] - previous["accuracy"][c]["rate"]}
                               for c in CONDITIONS}}
    for key, group in by_group.items():
        if key.startswith("qtype="):
            old = previous["exploratory"]["by_group"][key]["accuracy"]
            vs_previous[key] = {c: {"previous": old[c]["rate"], "now": group["accuracy"][c]["rate"],
                                    "delta": group["accuracy"][c]["rate"] - old[c]["rate"]} for c in CONDITIONS}

    snapshots = json.loads((EXPERIMENT / "env.json").read_text(encoding="utf-8"))["usage"]["snapshots"]
    usage = {"start": snapshots[0], "after_pilot": next((x for x in snapshots if x["label"] == "after-pilot"), None),
             "end": snapshots[-1]}
    summary = {
        "usage": usage,
        "flow": {"sessions": len(sessions), "status": dict(sorted(flow.items())),
                 "excluded_units_by_reason": dict(sorted(reasons.items())), "excluded_units": len(bad_units),
                 "analyzed_units": len(units), "analyzed_scenarios": len({u.split("-")[0] for u in units})},
        "accuracy": accuracy_table(scores, units),
        "confirmatory": comparisons,
        "exploratory": {"by_group": by_group, "by_subtype": by_subtype, "evidence_by_qtype": evidence_by_qtype, "evidence_in_packet": coverage,
                        "evidence_relation_mix": dict(sorted(relation_mix.items())), "mean_cost": cost,
                        "vs_previous": vs_previous},
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
    with (RESULTS / "tables" / "comparisons.csv").open("w", encoding="utf-8", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["hypothesis", "new", "old", "n", "b", "c", "diff", "boot_low", "boot_high",
                         "p_margin", "p_holm", "mcnemar_p", "verdict"])
        for name, c in comparisons.items():
            writer.writerow([name, c["new"], c["old"], c["n"], c["b"], c["c"], c["diff"], *c["bootstrap"]["ci95"],
                             c["bootstrap"]["p_margin"], c["p_holm"], c["mcnemar_p"], c["verdict"]])
    labels = {"H1": "judge-all − no-packet (A)", "H2": "judge-all − rrf-fallback (B)"}
    values = [{"comparison": labels[n], "diff": c["diff"] * 100, "low": c["bootstrap"]["ci95"][0] * 100,
               "high": c["bootstrap"]["ci95"][1] * 100} for n, c in comparisons.items()]
    comparison_axis = {"field": "comparison", "type": "nominal", "sort": list(labels.values()), "title": None,
                       "axis": {"labelLimit": 300}}
    n_units = next(iter(comparisons.values()))["n"]
    figure = {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": "정답률 차이", "subtitle": f"n={n_units}. 점은 차이, 막대는 시나리오 군집 부트스트랩 95% 신뢰구간, 기준선은 +10%p"},
        "width": 420,
        "height": 120,
        "data": {"values": values},
        "layer": [
            {"mark": {"type": "errorbar"},
             "encoding": {"y": comparison_axis,
                          "x": {"field": "low", "type": "quantitative", "title": "정답률 차이(%p)"},
                          "x2": {"field": "high"}}},
            {"mark": {"type": "point", "filled": True},
             "encoding": {"y": comparison_axis, "x": {"field": "diff", "type": "quantitative"}}},
            {"mark": "rule", "encoding": {"x": {"datum": 10}}},
        ],
    }
    (RESULTS / "figures" / "differences.vl.json").write_text(json.dumps(figure, ensure_ascii=False, indent=2) + "\n",
                                                            encoding="utf-8")


if __name__ == "__main__":
    main()
