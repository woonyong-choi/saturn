#!/usr/bin/env python3
"""처리: raw의 받는 쪽 답을 근거 사실마다 채점하고, 저장한 Saturn 이벤트로 변환 충실도 측정값을 낸다. raw만 읽는다.

사용: 02-process.py
조건 이름(기록을 만든 provider)은 채점에 쓰지 않고 출력 열로만 옮긴다.
"""
import csv
import glob
import json
import os
import sys

import common
import saturnview as view
import tasks

PROCESSED = os.path.join(common.EXP, "data", "processed")
RAW = os.path.join(common.EXP, "data", "raw")


def read_jsonl(path):
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def parse_answers(receiver, stdout):
    """받는 쪽 출력에서 답 JSON 객체를 꺼낸다. 못 읽으면 None."""
    text = ""
    if receiver == "claude":
        try:
            data = json.loads(stdout)
            text = data.get("result", "")
            if int(data.get("num_turns", 1)) > 1:
                return None, True
        except ValueError:
            return None, False
    else:
        for line in stdout.splitlines():
            try:
                message = json.loads(line)
            except ValueError:
                continue
            item = message.get("item") or {}
            if message.get("type") == "item.started" and item.get("type") not in ("agent_message", "reasoning", None):
                return None, True
            if message.get("type") == "item.completed" and item.get("type") == "agent_message":
                text = item.get("text", "")
    start, end = text.find("{"), text.rfind("}")
    try:
        answers = json.loads(text[start:end + 1])
    except ValueError:
        return None, False
    return (answers, False) if isinstance(answers, dict) else (None, False)


def write_csv(name, rows, fields):
    os.makedirs(PROCESSED, exist_ok=True)
    with open(os.path.join(PROCESSED, name), "w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def item_metrics(task, source, records, common_facts):
    item_facts = {}
    for fact in task["facts"]:
        item_facts.setdefault(fact["item"], []).append(fact)
    tools = [r for r in records if r["type"] == "tool"]
    rows = []
    for item in task["items"]:
        shared = [f for f in item_facts.get(item["id"], []) if f["id"] in common_facts]
        if item["kind"] != "FileEdit" and not shared:
            continue
        if item["kind"] == "FileEdit":
            hit = next((r for r in tools if r["kind"] == "FileEdit"), None)
        else:
            hit = next((r for r in tools if any(f["marker"] in (r["output"] or "") for f in shared)), None)
        row = {"task_id": task["task_id"], "provider": source, "item": item["id"], "truth_kind": item["kind"],
               "captured": int(hit is not None), "kind_ok": int(hit is not None and hit["kind"] == item["kind"]),
               "path_ok": "", "memo_ok": int(hit is not None and view.memo_fields_ok(item, hit, item_facts.get(item["id"], [])))}
        if item["path"]:
            row["path_ok"] = int(hit is not None and view.path_recovered(item, hit))
        rows.append(row)
    return rows


def main():
    packet_files = sorted(glob.glob(os.path.join(RAW, "packets-*.jsonl")))
    if not packet_files:
        print("02-process: data/raw에 packets 파일이 없다", file=sys.stderr)
        sys.exit(2)
    cfg = common.params()
    packets = {}
    for path in packet_files:
        for row in read_jsonl(path):
            packets[(row["condition"], row["task_id"])] = json.loads(row["line"])
    events = {}
    for path in sorted(glob.glob(os.path.join(RAW, "events-*.jsonl"))):
        for row in read_jsonl(path):
            events.setdefault((row["condition"], row["task_id"]), []).append(json.loads(row["line"]))
    trials, flow, items, packet_rows, activity = [], [], [], [], []
    for path in sorted(glob.glob(os.path.join(RAW, "receiver-*.jsonl"))):
        for row in read_jsonl(path):
            if row["dir"] != "out":
                continue
            source, receiver = row["condition"].split("->")
            task_id = row["task_id"]
            info = packets[(source, task_id)]
            payload = json.loads(row["line"])
            answers, tool_used = (None, False)
            if payload["exit_code"] == 0:
                answers, tool_used = parse_answers(receiver, payload["stdout"])
            status = "failed" if payload["exit_code"] != 0 else "tool_used" if tool_used else "unparsed" if answers is None else "ok"
            flow.append({"task_id": task_id, "source": source, "receiver": receiver, "status": status,
                         "attempt": payload.get("attempt", 1), "elapsed_s": payload.get("elapsed_s", "")})
            task = tasks.make_task(cfg["seed"], int(task_id[1:]))
            by_id = {f["id"]: f for f in task["facts"]}
            for fact_id in info["facts"]:
                given = (answers or {}).get(fact_id, "")
                correct = int(by_id[fact_id]["marker"].lower() in str(given).lower())
                trials.append({"task_id": task_id, "fact": fact_id, "source": source, "receiver": receiver,
                               "status": status, "correct": correct,
                               "in_packet": int(by_id[fact_id]["marker"] in info["packet"])})
    for (source, task_id), info in sorted(packets.items()):
        packet_rows.append({"task_id": task_id, "provider": source, "tokens": info["tokens"],
                            "included": len(info["included"]), "facts": len(info["facts"]), "budget": info["budget"]})
    for (source, task_id), evs in sorted(events.items()):
        task = tasks.make_task(cfg["seed"], int(task_id[1:]))
        records = view.build_records(evs)
        items.extend(item_metrics(task, source, records, set(packets[(source, task_id)]["facts"])))
        tools = [r for r in records if r["type"] == "tool"]
        activity.append({"task_id": task_id, "provider": source, "tools": len(tools),
                         "reading": sum(1 for r in tools if r["kind"] == "FileRead"),
                         "editing": sum(1 for r in tools if r["kind"] == "FileEdit"),
                         "shell": sum(1 for r in tools if r["kind"] in ("Shell", "TestRun")),
                         "reasoning_events": sum(1 for e in evs if "ToolCall" in e and e["ToolCall"]["detail"]["category"] == "Reasoning"),
                         "wrapped_command": sum(1 for r in tools if (r["command"] or "").startswith("/bin/zsh -lc")),
                         "edit_output_empty": sum(1 for r in tools if r["kind"] == "FileEdit" and not r["output"]),
                         "exit_code_missing": sum(1 for r in tools if r["kind"] in ("Shell", "TestRun") and r["exit_code"] is None)})
    write_csv("trials.csv", trials, ["task_id", "fact", "source", "receiver", "status", "correct", "in_packet"])
    write_csv("flow.csv", flow, ["task_id", "source", "receiver", "status", "attempt", "elapsed_s"])
    write_csv("items.csv", items, ["task_id", "provider", "item", "truth_kind", "captured", "kind_ok", "path_ok", "memo_ok"])
    write_csv("packets.csv", packet_rows, ["task_id", "provider", "tokens", "included", "facts", "budget"])
    write_csv("activity.csv", activity, ["task_id", "provider", "tools", "reading", "editing", "shell", "reasoning_events", "wrapped_command", "edit_output_empty", "exit_code_missing"])


if __name__ == "__main__":
    main()
