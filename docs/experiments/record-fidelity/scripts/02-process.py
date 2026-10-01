#!/usr/bin/env python3
"""1단계 처리: 원시 줄을 engine의 변환 코드로 재생해 Saturn 이벤트를 만들고, 근거 사실마다 측정값을 낸다.

사용: 02-process.py stage1 [--source pilot] [--facts-only]
  --source pilot   오케스트레이션 폴더의 사전 시험 원시 줄을 읽는다(저장소 data/raw 대신).
  --facts-only     두 provider가 모두 거친 근거 사실 수만 센다. 변환과 패킷을 만들지 않는다.
"""
import argparse
import csv
import glob
import json
import os
import subprocess
import sys

import common
import saturnview as view
import shim
import tasks

PACKET = os.path.join(common.REPO, "target", "debug", "examples", "packet")
REPLAY = os.path.join(common.ORCH, "replay")


def load_sessions(source):
    sessions = {}
    if source == "pilot":
        pairs = [(os.path.basename(p)[:-6], p) for p in sorted(glob.glob(os.path.join(common.ORCH, "pilot", "raw", "*.jsonl")))]
        for name, path in pairs:
            provider, task_id = name.split("-", 1)
            rows = []
            for line in open(path, encoding="utf-8"):
                row = json.loads(line)
                kept = shim.clean(row["dir"], row["line"])
                if kept is not None:
                    rows.append({"dir": row["dir"], "line": kept})
            sessions[(provider, task_id)] = {"rows": rows, "exit_code": 0}
        return sessions
    runs = {}
    paths = [p for provider in common.PROVIDERS for p in sorted(glob.glob(os.path.join(common.EXP, "data", "raw", f"{provider}-*.jsonl")))]
    for path in paths:
        for line in open(path, encoding="utf-8"):
            row = json.loads(line)
            runs.setdefault((row["condition"], row["task_id"]), {"rows": [], "exit_code": None, "run_id": row["run_id"]})
            entry = runs[(row["condition"], row["task_id"])]
            if row["dir"] == "meta":
                entry["exit_code"] = json.loads(row["line"])["exit_code"]
            else:
                entry["rows"].append({"dir": row["dir"], "line": row["line"]})
    return runs


def replay(provider, task_id, rows):
    os.makedirs(os.path.join(REPLAY, "work"), exist_ok=True)
    raw = os.path.join(REPLAY, f"{provider}-{task_id}.raw.jsonl")
    out = os.path.join(REPLAY, f"{provider}-{task_id}.events.jsonl")
    prompt = os.path.join(REPLAY, "prompt.txt")
    with open(raw, "w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    with open(prompt, "w", encoding="utf-8") as handle:
        handle.write("replay")
    env = dict(os.environ, SHIM_REPLAY=raw)
    subprocess.run(
        [common.EXAMPLE, provider, "--program", common.SHIM, "--workdir", os.path.join(REPLAY, "work"),
         "--prompt-file", prompt, "--out", out, "--timeout-s", "60"],
        env=env, check=True, capture_output=True,
    )
    return view.read_events(out)


def traversal(task, provider, rows):
    texts = view.raw_tool_results(provider, rows)
    facts = {fact["id"] for fact in task["facts"] if any(fact["marker"] in text for text in texts)}
    edited = any(path.endswith("config/app.cfg") for path in view.raw_edited_paths(provider, rows))
    return facts, edited


def questions_text(task):
    return "Answer from the earlier work:\n" + "\n".join(f"- {fact['question']}" for fact in task["facts"]) + "\n"


def scenario(task, records):
    rows = [{"seq": 1, "kind": "user", "text": task["instruction"]}]
    for number, record in enumerate(records, 2):
        if record["type"] == "agent":
            rows.append({"seq": number, "kind": "agent", "text": record["text"]})
        else:
            args = {"command": record["command"]} if record["command"] is not None else {}
            rows.append({"seq": number, "kind": "tool", "tool": view.TOOL_NAME[record["kind"]], "args": args, "result": record["output"]})
    rows.append({"seq": len(records) + 2, "kind": "user", "text": questions_text(task)})
    return rows


def make_packet(provider, task_id, task, records, evidence_seqs, budget, condition):
    base = os.path.join(REPLAY, f"{provider}-{task_id}")
    with open(base + ".scenario.jsonl", "w", encoding="utf-8") as handle:
        handle.write(json.dumps({"scenario_id": task_id, "record": scenario(task, records)}, ensure_ascii=False) + "\n")
    command = [PACKET, "--scenarios", base + ".scenario.jsonl", "--scenario-id", task_id,
               "--budget-tokens", str(budget), "--format", "json", "--condition", condition]
    if condition == "judge-only":
        with open(base + ".judgments.json", "w", encoding="utf-8") as handle:
            json.dump({"compact": [{"seq": seq, "probability": 1.0 if seq in evidence_seqs else 0.0} for seq in range(2, len(records) + 2)]}, handle)
        command += ["--judgments", base + ".judgments.json"]
    out = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def saturn_records(task, provider, rows, item_facts):
    """재생한 Saturn 기록, 항목별 대응 기록, oracle 판단이 남기는 기록 번호를 돌려준다."""
    records = view.build_records(replay(provider, task["task_id"], rows))
    tools = [(number, record) for number, record in enumerate(records, 2) if record["type"] == "tool"]
    evidence = set()
    associated = {}
    for item in task["items"]:
        if item["kind"] == "FileEdit":
            hit = next(((n, r) for n, r in tools if r["kind"] == "FileEdit"), None)
        else:
            markers = [fact["marker"] for fact in item_facts.get(item["id"], [])]
            hit = next(((n, r) for n, r in tools if any(m in (r["output"] or "") for m in markers)), None)
        associated[item["id"]] = hit
        if hit and item_facts.get(item["id"]):
            evidence.add(hit[0])
    return records, associated, evidence


def write_csv(path, rows, fields):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stage")
    parser.add_argument("--source", default="data")
    parser.add_argument("--facts-only", action="store_true")
    parser.add_argument("--budget", type=int, default=None)
    args = parser.parse_args()
    cfg = common.params()
    sessions = load_sessions(args.source)
    seed = 1990 if args.source == "pilot" else cfg["seed"]
    task_ids = sorted({task_id for _, task_id in sessions})
    out_dir = os.path.join(common.ORCH, "pilot", "processed") if args.source == "pilot" else os.path.join(common.EXP, "data", "processed")
    fact_rows, item_rows, packet_rows, flow = [], [], [], []
    if not args.facts_only:
        subprocess.run(["cargo", "build", "-q", "-p", "saturn-core", "--example", "packet"], cwd=common.REPO, check=True)
    for task_id in task_ids:
        task = tasks.make_task(seed, int(task_id[1:]))
        seen = {}
        for provider in common.PROVIDERS:
            session = sessions.get((provider, task_id))
            if session is None or session["exit_code"] != 0:
                flow.append({"task_id": task_id, "provider": provider, "status": "failed"})
                continue
            flow.append({"task_id": task_id, "provider": provider, "status": "ok"})
            seen[provider] = traversal(task, provider, session["rows"])
        both = len(seen) == 2
        common_facts = seen["codex"][0] & seen["claude"][0] if both else set()
        common_edit = seen["codex"][1] and seen["claude"][1] if both else False
        if args.facts_only:
            print(task_id, "common facts", len(common_facts), "edit", common_edit, {p: len(v[0]) for p, v in seen.items()})
            continue
        if not both:
            continue
        item_facts = {}
        for fact in task["facts"]:
            item_facts.setdefault(fact["item"], []).append(fact)
        for provider in common.PROVIDERS:
            records, associated, evidence = saturn_records(task, provider, sessions[(provider, task_id)]["rows"], item_facts)
            packets = {cond: make_packet(provider, task_id, task, records, evidence, args.budget or cfg["packet_budget_tokens"], cond)
                       for cond in ("rrf-only", "judge-only")}
            for cond, packet in packets.items():
                packet_rows.append({"task_id": task_id, "provider": provider, "condition": cond,
                                    "tokens": packet["tokens"], "included": len(packet["included"]), "records": len(records)})
            for item in task["items"]:
                if item["kind"] != "FileEdit" and not any(f["id"] in common_facts for f in item_facts.get(item["id"], [])):
                    continue
                if item["kind"] == "FileEdit" and not common_edit:
                    continue
                hit = associated[item["id"]]
                captured = hit is not None
                record = hit[1] if hit else None
                item_rows.append({
                    "task_id": task_id, "provider": provider, "item": item["id"], "truth_kind": item["kind"],
                    "captured": int(captured),
                    "kind_ok": int(captured and record["kind"] == item["kind"]),
                    "path_ok": int(captured and item["path"] is not None and view.path_recovered(item, record)) if item["path"] else "",
                    "memo_ok": int(captured and view.memo_fields_ok(item, record, item_facts.get(item["id"], []))),
                })
            for fact in task["facts"]:
                if fact["id"] not in common_facts:
                    continue
                marker = fact["marker"]
                row = {"task_id": task_id, "provider": provider, "fact": fact["id"], "item": fact["item"],
                       "captured": int(any(marker in (r.get("output") or "") for r in records if r["type"] == "tool"))}
                for cond, packet in packets.items():
                    row["in_packet_" + cond.replace("-", "_")] = int(marker in packet["packet"])
                fact_rows.append(row)
    write_csv(os.path.join(out_dir, "flow.csv"), flow, ["task_id", "provider", "status"])
    if args.facts_only:
        return
    write_csv(os.path.join(out_dir, "facts.csv"), fact_rows, ["task_id", "provider", "fact", "item", "captured", "in_packet_rrf_only", "in_packet_judge_only"])
    write_csv(os.path.join(out_dir, "items.csv"), item_rows, ["task_id", "provider", "item", "truth_kind", "captured", "kind_ok", "path_ok", "memo_ok"])
    write_csv(os.path.join(out_dir, "packets.csv"), packet_rows, ["task_id", "provider", "condition", "tokens", "included", "records"])


if __name__ == "__main__":
    main()
