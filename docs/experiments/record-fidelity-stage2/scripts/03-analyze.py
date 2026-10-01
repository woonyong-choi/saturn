#!/usr/bin/env python3
"""분석: C2 판정과 탐색 분석. processed의 CSV만 읽고 results/에 쓴다.

사용: 03-analyze.py
"""
import csv
import json
import os
from collections import defaultdict

import common
import stats

PROCESSED = os.path.join(common.EXP, "data", "processed")
RESULTS = os.path.join(common.EXP, "results")
RECEIVERS = ["claude", "codex"]


def read(name):
    with open(os.path.join(PROCESSED, name), encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


def rate(k, n):
    return {"k": k, "n": n, "rate": k / n if n else None, "wilson": stats.wilson(k, n)}


def measure(pairs_by_task, cfg):
    """pairs_by_task: {작업: [(Codex 기록, Claude 기록), ...]}. 차이는 Codex - Claude."""
    pairs = [p for task in sorted(pairs_by_task) for p in pairs_by_task[task]]
    n = len(pairs)
    table = stats.newcombe_paired(pairs)
    boot = stats.cluster_bootstrap(pairs_by_task, cfg["bootstrap_reps"], cfg["bootstrap_seed"]) if n else (None, None)
    return {"n": n, "tasks": len(pairs_by_task), "codex": rate(sum(x for x, _ in pairs), n),
            "claude": rate(sum(y for _, y in pairs), n), "paired": table, "bootstrap_ci95": list(boot),
            "verdict": stats.verdict(boot[0], boot[1], cfg["margin"]) if n else "보류",
            "verdict_newcombe": stats.verdict(table["lower"], table["upper"], cfg["margin"]) if n else "보류"}


def pairs_of(rows, field):
    values = defaultdict(dict)
    for row in rows:
        values[(row["task_id"], row["fact"])][row["source"]] = int(row[field])
    groups = defaultdict(list)
    for (task, _), v in sorted(values.items()):
        if len(v) == 2:
            groups[task].append((v["codex"], v["claude"]))
    return groups


def metric_pairs(rows, column):
    values = defaultdict(dict)
    for row in rows:
        if row[column] != "":
            values[(row["task_id"], row["item"])][row["provider"]] = int(row[column])
    groups = defaultdict(list)
    for (task, _), v in sorted(values.items()):
        if len(v) == 2:
            groups[task].append((v["codex"], v["claude"]))
    return groups


def main():
    cfg = common.params()
    trials, flow = read("trials.csv"), read("flow.csv")
    dropped = {(r["task_id"], r["receiver"]) for r in flow if r["status"] == "failed"}
    trials = [t for t in trials if (t["task_id"], t["receiver"]) not in dropped]
    c2 = {r: measure(pairs_of([t for t in trials if t["receiver"] == r], "correct"), cfg) for r in RECEIVERS}
    verdicts = [m["verdict"] for m in c2.values()]
    c2_verdict = "기각" if "기각" in verdicts else "채택" if all(v == "채택" for v in verdicts) else "보류"
    pooled_groups = defaultdict(list)
    for r in RECEIVERS:
        for task, pairs in pairs_of([t for t in trials if t["receiver"] == r], "correct").items():
            pooled_groups[task].extend(pairs)
    pooled = measure(pooled_groups, cfg)
    by_source = {}
    for r in RECEIVERS + ["all"]:
        for s in common.PROVIDERS:
            rows = [t for t in trials if t["source"] == s and (r == "all" or t["receiver"] == r)]
            by_source[f"receiver={r},source={s}"] = rate(sum(int(t["correct"]) for t in rows), len(rows))
    packet_in = {s: rate(sum(int(t["in_packet"]) for t in trials if t["source"] == s and t["receiver"] == "claude"),
                         sum(1 for t in trials if t["source"] == s and t["receiver"] == "claude")) for s in common.PROVIDERS}
    items, packets, activity = read("items.csv"), read("packets.csv"), read("activity.csv")
    conversion = {}
    for name, column in (("capture", "captured"), ("kind", "kind_ok"), ("path", "path_ok"), ("memo", "memo_ok")):
        conversion[name] = measure(metric_pairs(items, column), cfg)
    totals = defaultdict(lambda: defaultdict(int))
    for row in activity:
        for key, value in row.items():
            if key not in ("task_id", "provider"):
                totals[row["provider"]][key] += int(value)
    status = defaultdict(int)
    for row in flow:
        status[row["status"]] += 1
    mean_tokens = {s: sum(int(p["tokens"]) for p in packets if p["provider"] == s) / max(1, sum(1 for p in packets if p["provider"] == s))
                   for s in common.PROVIDERS}
    env = common.params()
    with open(os.path.join(common.EXP, "env.json"), encoding="utf-8") as handle:
        snapshots = json.load(handle)["usage"]["snapshots"]
    summary = {
        "margin": cfg["margin"], "c2": {"verdict": c2_verdict, "receivers": c2, "pooled_descriptive": pooled},
        "flow": {"sessions": len(flow), "dropped_task_receivers": len(dropped), "status": dict(sorted(status.items())), "tasks": len({r["task_id"] for r in flow}),
                 "common_facts": len({(t["task_id"], t["fact"]) for t in trials})},
        "exploratory": {"accuracy_by_source": by_source, "fact_in_packet_by_source": packet_in,
                        "conversion": conversion, "activity": {k: dict(v) for k, v in totals.items()},
                        "mean_packet_tokens": mean_tokens, "packet_budget": cfg["packet_budget_tokens"]},
        "usage": {"start": snapshots[0] if snapshots else None, "end": snapshots[-1] if snapshots else None},
    }
    os.makedirs(os.path.join(RESULTS, "tables"), exist_ok=True)
    os.makedirs(os.path.join(RESULTS, "figures"), exist_ok=True)
    with open(os.path.join(RESULTS, "summary.json"), "w", encoding="utf-8") as handle:
        json.dump(summary, handle, ensure_ascii=False, indent=2, sort_keys=True)
        handle.write("\n")
    with open(os.path.join(RESULTS, "tables", "c2.csv"), "w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["receiver", "n", "codex_k", "claude_k", "diff", "boot_low", "boot_high", "newcombe_low", "newcombe_high", "verdict"])
        for r, m in {**c2, "pooled": pooled}.items():
            writer.writerow([r, m["n"], m["codex"]["k"], m["claude"]["k"], round(m["paired"]["diff"], 4), *[round(v, 4) for v in m["bootstrap_ci95"]],
                             round(m["paired"]["lower"], 4), round(m["paired"]["upper"], 4), m["verdict"]])
    values = [{"receiver": f"받는 쪽 {r}", "diff": m["paired"]["diff"] * 100, "low": m["bootstrap_ci95"][0] * 100,
               "high": m["bootstrap_ci95"][1] * 100} for r, m in c2.items()]
    axis = {"field": "receiver", "type": "nominal", "title": None, "axis": {"labelLimit": 300}}
    figure = {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": "정답률 차이(Codex 기록 − Claude 기록)",
                  "subtitle": "점은 차이, 막대는 작업 군집 부트스트랩 95% 신뢰구간, 기준선은 −10%p"},
        "width": 420, "height": 100, "data": {"values": values},
        "layer": [
            {"mark": {"type": "errorbar"}, "encoding": {"y": axis, "x": {"field": "low", "type": "quantitative", "title": "정답률 차이(%p)", "scale": {"domain": [-15, 15]}}, "x2": {"field": "high"}}},
            {"mark": {"type": "point", "filled": True}, "encoding": {"y": axis, "x": {"field": "diff", "type": "quantitative"}}},
            {"mark": "rule", "encoding": {"x": {"datum": -10}}},
        ],
    }
    with open(os.path.join(RESULTS, "figures", "differences.vl.json"), "w", encoding="utf-8") as handle:
        json.dump(figure, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    print(c2_verdict, {r: (m["verdict"], round(m["paired"]["diff"], 3), m["bootstrap_ci95"]) for r, m in c2.items()})


if __name__ == "__main__":
    main()
