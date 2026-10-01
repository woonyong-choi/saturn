#!/usr/bin/env python3
"""1단계 분석: C1 판정과 2단계 진행 조건 판정. processed의 CSV만 읽고 results/summary.json과 표를 쓴다.

사용: 03-analyze.py stage1|stage2 [--processed DIR] [--out DIR]
"""
import argparse
import csv
import glob
import json
import os
from collections import defaultdict

import common
import stats
import tasks

METRICS = [
    ("capture", "facts.csv", "captured", "근거 항목 포착률"),
    ("kind", "items.csv", "kind_ok", "도구 종류 변환 정확도"),
    ("path", "items.csv", "path_ok", "파일 경로 추출 정확도"),
    ("memo", "items.csv", "memo_ok", "메모 필드 정확도"),
]
PACKET_METRICS = [
    ("packet_rrf", "facts.csv", "in_packet_rrf_only", "RRF 패킷 포함률"),
    ("packet_oracle", "facts.csv", "in_packet_judge_only", "oracle 패킷 포함률"),
]


def read(path):
    with open(path, encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


def unit_key(row):
    return (row["task_id"], row.get("fact") or row["item"])


def paired(rows, column):
    values = defaultdict(dict)
    for row in rows:
        if row[column] == "":
            continue
        values[unit_key(row)][row["provider"]] = int(row[column])
    units = {key: (v["codex"], v["claude"]) for key, v in values.items() if len(v) == 2}
    groups = defaultdict(list)
    for (task, _), pair in sorted(units.items()):
        groups[task].append(pair)
    return list(units.values()), groups


def measure(rows, column, label, cfg, margin):
    pairs, groups = paired(rows, column)
    codex_k = sum(x for x, _ in pairs)
    claude_k = sum(y for _, y in pairs)
    n = len(pairs)
    table = stats.newcombe_paired(pairs)
    boot = stats.cluster_bootstrap(groups, cfg["bootstrap_reps"], cfg["seed"]) if n else (float("nan"),) * 2
    main = stats.verdict(table["lower"], table["upper"], margin) if n else "보류"
    boot_verdict = stats.verdict(boot[0], boot[1], margin) if n else "보류"
    return {
        "label": label, "n": n,
        "codex": {"k": codex_k, "rate": codex_k / n if n else None, "wilson": stats.wilson(codex_k, n)},
        "claude": {"k": claude_k, "rate": claude_k / n if n else None, "wilson": stats.wilson(claude_k, n)},
        "paired": table, "bootstrap_ci": boot,
        "verdict_newcombe": main, "verdict_bootstrap": boot_verdict,
        "verdict": main if main == boot_verdict else "보류",
    }


def parse_answers(receiver, stdout):
    """받는 쪽 출력에서 답 JSON 객체를 꺼낸다. 못 읽으면 빈 객체."""
    text = ""
    if receiver == "claude":
        try:
            text = json.loads(stdout).get("result", "")
        except ValueError:
            text = stdout
    else:
        for line in stdout.splitlines():
            try:
                message = json.loads(line)
            except ValueError:
                continue
            item = message.get("item") or {}
            if message.get("type") == "item.completed" and item.get("type") == "agent_message":
                text = item.get("text", "")
    start, end = text.find("{"), text.rfind("}")
    try:
        answers = json.loads(text[start:end + 1])
    except ValueError:
        return {}
    return answers if isinstance(answers, dict) else {}


def analyze_stage2(args, cfg):
    margin = cfg["margin_stage2"]
    shared = defaultdict(set)
    for row in read(os.path.join(args.processed, "facts.csv")):
        shared[row["task_id"]].add(row["fact"])
    answers = {}
    for path in sorted(glob.glob(os.path.join(common.EXP, "data", "raw", "receiver-*.jsonl"))):
        for line in open(path, encoding="utf-8"):
            row = json.loads(line)
            if row["dir"] != "out":
                continue
            source, receiver = row["condition"].split("->")
            payload = json.loads(row["line"])
            answers[(row["task_id"], source, receiver)] = parse_answers(receiver, payload["stdout"]) if payload["exit_code"] == 0 else {}
    result = {}
    for receiver in common.PROVIDERS:
        rows = []
        for (task_id, source, got), given in answers.items():
            if got != receiver:
                continue
            task = tasks.make_task(cfg["seed"], int(task_id[1:]))
            for fact in task["facts"]:
                if fact["id"] in shared[task_id]:
                    correct = int(fact["marker"].lower() in str(given.get(fact["id"], "")).lower())
                    rows.append({"task_id": task_id, "fact": fact["id"], "provider": source, "correct": str(correct)})
        result[receiver] = measure(rows, "correct", f"받는 쪽 {receiver} 정답률", cfg, margin)
    verdicts = [m["verdict"] for m in result.values()]
    c2 = "기각" if "기각" in verdicts else "채택" if all(v == "채택" for v in verdicts) else "보류"
    path = os.path.join(args.out, "summary-stage2.json")
    with open(path, "w", encoding="utf-8") as handle:
        json.dump({"stage": 2, "margin": margin, "c2": {"verdict": c2, "receivers": result}}, handle, ensure_ascii=False, indent=1, sort_keys=True)
        handle.write("\n")
    print(c2, {k: (v["verdict"], round(v["paired"]["diff"], 3)) for k, v in result.items()})


def stage2_route(c1_verdict, enough_facts):
    if c1_verdict == "기각":
        return "defer-until-fix"
    return "run" if enough_facts else "skip"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stage")
    parser.add_argument("--processed", default=os.path.join(common.EXP, "data", "processed"))
    parser.add_argument("--out", default=os.path.join(common.EXP, "results"))
    args = parser.parse_args()
    cfg = common.params()
    if args.stage == "stage2":
        return analyze_stage2(args, cfg)
    margin = cfg["noninferiority_margin_stage1"]
    data = {name: read(os.path.join(args.processed, name)) for name in ("facts.csv", "items.csv", "flow.csv")}
    c1 = {name: measure(data[file], column, label, cfg, margin) for name, file, column, label in METRICS}
    descriptive = {name: measure(data[file], column, label, cfg, margin) for name, file, column, label in PACKET_METRICS}
    verdicts = [m["verdict"] for m in c1.values()]
    c1_verdict = "기각" if "기각" in verdicts else "채택" if all(v == "채택" for v in verdicts) else "보류"
    per_task = defaultdict(set)
    for row in data["facts.csv"]:
        per_task[row["task_id"]].add(row["fact"])
    total_common = sum(len(v) for v in per_task.values())
    mean_common = total_common / len(per_task) if per_task else 0.0
    enough = (mean_common >= cfg["gate_min_facts_per_task"] and total_common >= cfg["gate_min_facts_total"] and len(per_task) >= 20)
    totals = defaultdict(lambda: defaultdict(int))
    for row in read(os.path.join(args.processed, "activity.csv")):
        for key, value in row.items():
            if key not in ("task_id", "provider"):
                totals[row["provider"]][key] += int(value)
    failed = [row for row in data["flow.csv"] if row["status"] != "ok"]
    summary = {
        "stage": 1, "margin": margin, "c1": {"verdict": c1_verdict, "metrics": c1},
        "descriptive": descriptive,
        "activity": {provider: dict(values) for provider, values in totals.items()},
        "flow": {"sessions": len(data["flow.csv"]), "failed": len(failed), "tasks_evaluable": len(per_task)},
        "gate": {"a_c1_rejected": c1_verdict == "기각", "b_enough_facts": enough, "common_facts_total": total_common,
                 "common_facts_mean_per_task": mean_common, "stage2": stage2_route(c1_verdict, enough)},
    }
    os.makedirs(os.path.join(args.out, "tables"), exist_ok=True)
    with open(os.path.join(args.out, "summary.json"), "w", encoding="utf-8") as handle:
        json.dump(summary, handle, ensure_ascii=False, indent=1, sort_keys=True)
        handle.write("\n")
    with open(os.path.join(args.out, "tables", "stage1.csv"), "w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["metric", "n", "codex_k", "claude_k", "diff", "newcombe_lower", "newcombe_upper", "bootstrap_lower", "bootstrap_upper", "verdict"])
        for name, m in {**c1, **descriptive}.items():
            writer.writerow([name, m["n"], m["codex"]["k"], m["claude"]["k"], round(m["paired"]["diff"], 4),
                             round(m["paired"]["lower"], 4), round(m["paired"]["upper"], 4),
                             round(m["bootstrap_ci"][0], 4), round(m["bootstrap_ci"][1], 4), m["verdict"]])
    with open(os.path.join(args.out, "stage1-gate.json"), "w", encoding="utf-8") as handle:
        json.dump({"stage2": summary["gate"]["stage2"], "c1": c1_verdict}, handle)
        handle.write("\n")
    print(json.dumps(summary["gate"], ensure_ascii=False), c1_verdict, {k: (v["verdict"], round(v["paired"]["diff"], 3)) for k, v in c1.items()})


if __name__ == "__main__":
    main()
