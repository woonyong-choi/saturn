"""processed CSV에서 summary.json과 표를 재생성한다."""
from __future__ import annotations

import csv
import json
import math
import random
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROCESSED = ROOT / "data" / "processed"
RESULTS = ROOT / "results"
SEED = 252
BOOTSTRAP = 10000


def read(name: str) -> list[dict]:
    path = PROCESSED / f"{name}.csv"
    if not path.exists():
        return []
    with path.open(encoding="utf-8", newline="") as source:
        return list(csv.DictReader(source))


def wilson(k: int, n: int) -> list[float | None]:
    if not n:
        return [None, None]
    z = 1.959963984540054
    p = k / n
    centre = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return [max(0.0, centre - half), min(1.0, centre + half)]


def bootstrap(values: dict[str, float], clusters: dict[str, str]) -> list[float | None]:
    grouped = defaultdict(list)
    for key, value in values.items():
        grouped[clusters[key]].append(value)
    groups = list(grouped.values())
    if not groups:
        return [None, None]
    rng = random.Random(SEED)
    estimates = []
    for _ in range(BOOTSTRAP):
        picked = [groups[rng.randrange(len(groups))] for _ in groups]
        estimates.append(sum(sum(values) for values in picked) / sum(len(values) for values in picked))
    estimates.sort()
    return [estimates[int(0.025 * BOOTSTRAP)], estimates[int(0.975 * BOOTSTRAP) - 1]]


def paired_binary(rows: list[dict], left: str, right: str) -> dict:
    pair = defaultdict(dict)
    for row in rows:
        pair[(row["scenario_id"], row["provider"], row["qid"])][row["condition"]] = int(row["correct"])
    usable_keys = [key for key, values in pair.items() if left in values and right in values]
    usable = [pair[key] for key in usable_keys]
    lk = sum(values[left] for values in usable)
    rk = sum(values[right] for values in usable)
    diffs = {str(index): values[left] - values[right] for index, values in enumerate(usable)}
    clusters = {str(index): key[0] for index, key in enumerate(usable_keys)}
    ci = bootstrap(diffs, clusters)
    b = sum(values[left] == 1 and values[right] == 0 for values in usable)
    c = sum(values[left] == 0 and values[right] == 1 for values in usable)
    return {"left": left, "right": right, "left_k": lk, "right_k": rk, "n": len(usable),
            "left_rate": lk / len(usable) if usable else None, "right_rate": rk / len(usable) if usable else None,
            "difference": (lk - rk) / len(usable) if usable else None, "ci95": ci, "b": b, "c": c}


def token_summary(rows: list[dict]) -> dict:
    pairs = defaultdict(dict)
    for row in rows:
        pairs[(row["scenario_id"], row["provider"])][row["condition"]] = int(row["packet_tokens"])
    values = {"|".join(key): (value.get("tail-preserve", 0) - value.get("oldest-first", 0)) / value.get("oldest-first", 1)
              for key, value in pairs.items() if "tail-preserve" in value and "oldest-first" in value and value["oldest-first"]}
    clusters = {key: key.split("|")[0] for key in values}
    ordered = sorted(values.values())
    return {"n": len(values), "mean": sum(ordered) / len(ordered) if ordered else None,
            "sd": (sum((x - sum(ordered) / len(ordered)) ** 2 for x in ordered) / (len(ordered) - 1)) ** 0.5 if len(ordered) > 1 else None,
            "median": ordered[len(ordered) // 2] if ordered else None,
            "p5": ordered[max(0, int(len(ordered) * 0.05) - 1)] if ordered else None,
            "p95": ordered[min(len(ordered) - 1, int(len(ordered) * 0.95))] if ordered else None,
            "ci95": bootstrap(values, clusters)}


def proportion(rows: list[dict], measure: str, condition: str | None = None) -> dict:
    selected = [row for row in rows if condition is None or row.get("condition") == condition]
    k = sum(int(row[measure]) for row in selected)
    return {"k": k, "n": len(selected), "rate": k / len(selected) if selected else None,
            "ci95": wilson(k, len(selected))}


def provider_calls(*groups: list[dict]) -> dict[str, int]:
    counts = defaultdict(int)
    for group in groups:
        for row in group:
            counts[row.get("provider", "unknown")] += 1
    return dict(sorted(counts.items()))


def calls_by_provider(rows: list[dict]) -> dict[str, int]:
    return provider_calls(rows)


def manipulation_checks(rows: list[dict]) -> list[dict]:
    result = {}
    for row in rows:
        try:
            value = json.loads(row.get("manipulation_check_json", "{}"))
        except json.JSONDecodeError:
            continue
        if value.get("scenario_id"):
            result[value["scenario_id"]] = value
    return [result[key] for key in sorted(result)]


def main():
    questions = read("exp1_questions")
    trials = read("exp1_trials")
    exp2 = read("exp2_trials")
    requests = read("exp3_requests")
    stops = read("exp4_stops")
    RESULTS.mkdir(parents=True, exist_ok=True)
    accuracy = {condition: proportion(questions, "correct", condition) for condition in ("oldest-first", "tail-preserve")}
    comparison = paired_binary(questions, "tail-preserve", "oldest-first")
    exp3_calls = [{"provider": provider, "trial_id": trial_id}
                  for provider, trial_id in sorted({(r["provider"], r["trial_id"]) for r in requests})]
    summary = {"seed": SEED, "bootstrap_reps": BOOTSTRAP,
               "provider_calls": provider_calls(trials, exp2, exp3_calls, stops),
               "provider_calls_by_experiment": {
                   "exp1": calls_by_provider(trials),
                   "exp2": calls_by_provider(exp2),
                   "exp3": calls_by_provider(exp3_calls),
                   "exp4": calls_by_provider(stops),
               },
               "flow": {"exp1": len(trials), "exp1_questions": len(questions), "exp2": len(exp2), "exp3": len(requests), "exp4": len(stops)},
               "exp1": {"accuracy": accuracy, "comparison": comparison, "tokens": token_summary(trials),
                        "manipulation_check": manipulation_checks(trials)},
               "exp2": {"state_checked_first": {condition: proportion(exp2, "state_checked_first", condition) for condition in ("no-warning", "state-warning")},
                        "duplicate_touch": {condition: proportion(exp2, "duplicate_touch", condition) for condition in ("no-warning", "state-warning")}},
               "exp3": {"requests": requests, "by_method": {method: proportion([r for r in requests if r["request_method"] == method], "round_trip") for method in sorted({r["request_method"] for r in requests})}},
               "exp4": {"stops": stops, "by_provider_kind": {f"{provider}-{kind}": [r for r in stops if r["provider"] == provider and r["stop_kind"] == kind]
                                                    for provider in ("claude", "codex") for kind in ("interrupt", "force-kill")}}}
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    with (RESULTS / "tables.csv").open("w", encoding="utf-8", newline="") as target:
        writer = csv.writer(target)
        writer.writerow(["metric", "condition", "k", "n", "rate", "ci95_low", "ci95_high"])
        for name, table in (("accuracy", accuracy),):
            for condition, value in table.items():
                writer.writerow([name, condition, value["k"], value["n"], value["rate"], *value["ci95"]])


if __name__ == "__main__":
    main()
