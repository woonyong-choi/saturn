"""processed/의 채점 결과로 가설을 판정하고 results/에 쓴다."""
from __future__ import annotations

import csv
import json
import math
import random
from collections import defaultdict
from pathlib import Path

SEED = 249
BOOTSTRAP_REPS = 10_000
ALPHA = 0.05
H1_MARGIN = 0.15
H2_MARGIN = 0.05
Z95 = 1.959963984540054
EXPERIMENT = Path(__file__).resolve().parent.parent
PROCESSED = EXPERIMENT / "data" / "processed"
RESULTS = EXPERIMENT / "results"
CONDITIONS = ["last-input", "rule", "summary"]
COMPARISONS = {"H1": ("rule", "last-input"), "H2": ("summary", "rule")}


def read_csv(name: str) -> list[dict]:
    with (PROCESSED / f"{name}.csv").open(encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def wilson(k: int, n: int) -> list[float | None]:
    if n == 0:
        return [None, None]
    p = k / n
    centre = (p + Z95**2 / (2 * n)) / (1 + Z95**2 / n)
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n * n)) / (1 + Z95**2 / n)
    return [max(0.0, centre - half), min(1.0, centre + half)]


def paired(scores: dict, new: str, old: str, units: list[str]) -> dict:
    b = c = 0
    for unit in units:
        b += int(scores[unit][new] and not scores[unit][old])
        c += int(scores[unit][old] and not scores[unit][new])
    n = len(units)
    p = 1.0 if b + c == 0 else math.erfc(math.sqrt((b - c) ** 2 / (b + c) / 2))
    return {"n": n, "b": b, "c": c, "diff": (b - c) / n if n else None, "mcnemar_p": p}


def bootstrap(scores: dict, new: str, old: str, units: list[str], margin: float) -> dict:
    clusters = defaultdict(list)
    for unit in units:
        clusters[unit.split("-")[0]].append(scores[unit][new] - scores[unit][old])
    values = list(clusters.values())
    rng = random.Random(SEED)
    diffs = []
    for _ in range(BOOTSTRAP_REPS):
        picked = [values[rng.randrange(len(values))] for _ in values]
        diffs.append(sum(sum(group) for group in picked) / sum(len(group) for group in picked))
    diffs.sort()
    below = sum(diff <= margin for diff in diffs)
    above = sum(diff >= margin for diff in diffs)
    p = min(1.0, 2 * (min(below, above) + 1) / (BOOTSTRAP_REPS + 1))
    return {"clusters": len(values), "ci95": [diffs[250], diffs[9749]], "p_margin": p}


def holm(pvalues: dict[str, float]) -> dict[str, float]:
    adjusted, running = {}, 0.0
    for rank, name in enumerate(sorted(pvalues, key=pvalues.get)):
        running = max(running, min(1.0, (len(pvalues) - rank) * pvalues[name]))
        adjusted[name] = running
    return adjusted


def accuracy(scores: dict, units: list[str]) -> dict:
    return {
        condition: {"k": sum(scores[u][condition] for u in units), "n": len(units),
                    "rate": sum(scores[u][condition] for u in units) / len(units) if units else None,
                    "ci95": wilson(sum(scores[u][condition] for u in units), len(units))}
        for condition in CONDITIONS
    }


def verdict(name: str, ci: list[float], p: float) -> str:
    margin = H1_MARGIN if name == "H1" else H2_MARGIN
    if p > ALPHA:
        return "보류"
    if name == "H1":
        return "채택" if ci[0] >= margin else "기각" if ci[1] < margin else "보류"
    return "채택" if ci[1] <= margin else "기각" if ci[0] > margin else "보류"


def mean(values: list[float]) -> float | None:
    return sum(values) / len(values) if values else None


def main() -> None:
    sessions = read_csv("sessions")
    trials = read_csv("trials")
    bad = {(row["scenario_id"], row["provider"]) for row in sessions if row["status"] != "ok"}
    scores, meta = defaultdict(dict), {}
    for row in trials:
        if (row["scenario_id"], row["provider"]) in bad or row["correct"] == "":
            continue
        unit = row["unit"]
        scores[unit][row["condition"]] = int(row["correct"])
        meta[unit] = row
    units = sorted(unit for unit in scores if all(condition in scores[unit] for condition in CONDITIONS))
    summary_accuracy = accuracy(scores, units)
    comparisons = {}
    for name, (new, old) in COMPARISONS.items():
        margin = H1_MARGIN if name == "H1" else H2_MARGIN
        comp = paired(scores, new, old, units)
        comp["new"], comp["old"] = new, old
        comp["bootstrap"] = bootstrap(scores, new, old, units, margin)
        comparisons[name] = comp
    adjusted = holm({name: comp["bootstrap"]["p_margin"] for name, comp in comparisons.items()})
    for name, comp in comparisons.items():
        comp["p_holm"] = adjusted[name]
        comp["verdict"] = verdict(name, comp["bootstrap"]["ci95"], adjusted[name])

    by_group = {}
    for field in ("qtype", "provider"):
        for value in sorted({meta[u][field] for u in units}):
            subset = [u for u in units if meta[u][field] == value]
            by_group[f"{field}={value}"] = accuracy(scores, subset)
    evidence = read_csv("evidence")
    coverage = {}
    for condition in CONDITIONS:
        rows = [row for row in evidence if row["condition"] == condition]
        k = sum(int(row["in_packet"]) for row in rows)
        coverage[condition] = {"k": k, "n": len(rows), "rate": k / len(rows) if rows else None,
                               "ci95": wilson(k, len(rows))}

    packet_rows = read_csv("packets")
    session_rows = read_csv("sessions")
    cost = {}
    for condition in CONDITIONS:
        packets = [row for row in packet_rows if row["condition"] == condition]
        provider = defaultdict(float)
        for row in session_rows:
            if row["condition"] == condition and row["status"] == "ok":
                provider[row["scenario_id"]] += float(row["prompt_tokens"])
        totals = [provider[row["scenario_id"]] + (float(row["summary_tokens"]) if condition == "summary" else 0)
                  for row in packets if row["scenario_id"] in provider]
        cost[condition] = {"scenarios": len(totals), "total_tokens_sum": sum(totals),
                           "total_tokens_mean": mean(totals),
                           "packet_tokens_mean": mean([float(row["packet_tokens"]) for row in packets]),
                           "summary_tokens_mean": mean([float(row["summary_tokens"]) for row in packets])}

    flow = defaultdict(int)
    for row in sessions:
        flow[row["status"]] += 1
    summary = {
        "flow": {"sessions": len(sessions), "status": dict(sorted(flow.items())),
                 "excluded_units": len(bad), "analyzed_units": len(units),
                 "analyzed_scenarios": len({meta[u]["scenario_id"] for u in units})},
        "accuracy": summary_accuracy,
        "confirmatory": comparisons,
        "exploratory": {"by_group": by_group, "evidence_in_packet": coverage, "cost": cost},
        "limits": {"provider_calls": 150, "summary_calls": 30},
    }
    (RESULTS / "tables").mkdir(parents=True, exist_ok=True)
    (RESULTS / "figures").mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                                          encoding="utf-8")
    with (RESULTS / "tables" / "accuracy.csv").open("w", encoding="utf-8", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["condition", "k", "n", "rate", "ci95_low", "ci95_high"])
        for condition, row in summary_accuracy.items():
            writer.writerow([condition, row["k"], row["n"], row["rate"], *row["ci95"]])
    with (RESULTS / "tables" / "comparisons.csv").open("w", encoding="utf-8", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["hypothesis", "new", "old", "n", "b", "c", "diff", "ci95_low", "ci95_high",
                         "p_holm", "verdict"])
        for name, row in comparisons.items():
            writer.writerow([name, row["new"], row["old"], row["n"], row["b"], row["c"], row["diff"],
                             *row["bootstrap"]["ci95"], row["p_holm"], row["verdict"]])


if __name__ == "__main__":
    main()
