#!/usr/bin/env python3
"""수집: 저장한 provider 원시 줄을 수정된 변환으로 재생해 Saturn 이벤트를 만들고, 패킷을 만들어 받는 쪽 새 session의 답을 저장한다.

사용: 01-collect.py [--dry-run] [--budget N] [--limit-tasks N] [--resume 실행 id]
  --dry-run       재생과 패킷 생성만 하고 오케스트레이션 폴더에 요약을 쓴다. provider를 부르지 않고 저장소에 쓰지 않는다.
  --budget N      패킷 예산(토큰). 없으면 env.json의 `packet_budget_tokens`.
  --limit-tasks N 시드로 섞은 순서에서 앞의 N개 작업만 받는 쪽에 넘긴다(사용량 시험용).
  --resume ID     같은 실행 id의 raw 파일에 이어 쓰고 끝난 세션은 건너뛴다.
조건이 갖춰지지 않으면 원인 한 줄을 쓰고 종료 코드 2, 근거 사실이 모자라 받는 쪽을 돌리지 않으면 4, 사용률 한도에 닿으면 3으로 끝난다.
provider 동시 실행은 받는 쪽마다 하나, 합쳐 둘 이하다.
"""
import argparse
import datetime
import glob
import json
import os
import random
import subprocess
import sys
import threading
import time

import common
import saturnview as view
import tasks
import usage

RECEIVER_PROMPT = (
    "Below is context handed over from earlier work in a project. Use only this context and do not use any tools. "
    "Answer each question with the exact value, or `unknown` if the context does not contain it. "
    "Reply with a single JSON object that maps each question id to its answer, and nothing else.\n\n"
    "=== context ===\n{packet}\n=== end of context ===\n\nQuestions:\n{questions}\n"
)
RECEIVER_COMMAND = {
    "claude": ["claude", "-p", "--model", "claude-opus-5-5", "--safe-mode", "--tools", "", "--no-session-persistence", "--output-format", "json"],
    "codex": ["codex", "exec", "-m", "gpt-6-sol", "--ephemeral", "--ignore-user-config", "--ignore-rules", "--sandbox", "read-only", "--skip-git-repo-check", "--json", "-"],
}
BATCH_TASKS = 4
MAX_CONSECUTIVE_FAILURES = 5


def fail(reason, code=2):
    print(f"01-collect: {reason}", file=sys.stderr)
    sys.exit(code)


def now_utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def load_sessions():
    runs = {}
    for provider in common.PROVIDERS:
        for path in sorted(glob.glob(os.path.join(common.SOURCE_RAW, f"{provider}-*.jsonl"))):
            for line in open(path, encoding="utf-8"):
                row = json.loads(line)
                entry = runs.setdefault((row["condition"], row["task_id"]), {"rows": [], "exit_code": None})
                if row["dir"] == "meta":
                    entry["exit_code"] = json.loads(row["line"])["exit_code"]
                else:
                    entry["rows"].append({"dir": row["dir"], "line": row["line"]})
    return runs


def traversal(task, provider, rows):
    texts = view.raw_tool_results(provider, rows)
    return {fact["id"] for fact in task["facts"] if any(fact["marker"] in text for text in texts)}


def replay(provider, task_id, rows):
    base = os.path.join(common.ORCH, "replay")
    os.makedirs(os.path.join(base, "work"), exist_ok=True)
    raw = os.path.join(base, f"{provider}-{task_id}.raw.jsonl")
    out = os.path.join(base, f"{provider}-{task_id}.events.jsonl")
    prompt = os.path.join(base, "prompt.txt")
    with open(raw, "w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    with open(prompt, "w", encoding="utf-8") as handle:
        handle.write("replay")
    subprocess.run(
        [common.EXAMPLE, provider, "--program", common.SHIM, "--workdir", os.path.join(base, "work"),
         "--prompt-file", prompt, "--out", out, "--timeout-s", "60"],
        env=dict(os.environ, SHIM_REPLAY=raw), check=True, capture_output=True,
    )
    with open(out, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def questions_text(facts):
    return "Answer from the earlier work:\n" + "\n".join(f"- {fact['question']}" for fact in facts) + "\n"


def make_packet(source, task, facts, records, evidence, budget):
    base = os.path.join(common.ORCH, "replay", f"{source}-{task['task_id']}")
    rows = [{"seq": 1, "kind": "user", "text": task["instruction"]}]
    for number, record in enumerate(records, 2):
        if record["type"] == "agent":
            rows.append({"seq": number, "kind": "agent", "text": record["text"]})
        else:
            rows.append({"seq": number, "kind": "tool", "tool": view.TOOL_NAME[record["kind"]],
                         "args": view.tool_args(record), "result": record["output"]})
    rows.append({"seq": len(records) + 2, "kind": "user", "text": questions_text(facts)})
    with open(base + ".scenario.jsonl", "w", encoding="utf-8") as handle:
        handle.write(json.dumps({"scenario_id": task["task_id"], "record": rows}, ensure_ascii=False) + "\n")
    with open(base + ".judgments.json", "w", encoding="utf-8") as handle:
        json.dump({"compact": [{"seq": seq, "probability": 1.0 if seq in evidence else 0.0} for seq in range(2, len(records) + 2)]}, handle)
    out = subprocess.run(
        [common.PACKET, "--scenarios", base + ".scenario.jsonl", "--scenario-id", task["task_id"],
         "--budget-tokens", str(budget), "--format", "json", "--condition", "judge-only",
         "--judgments", base + ".judgments.json"],
        check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def prepare(sessions, budget):
    """작업마다 두 provider가 모두 거친 근거 사실, 재생한 이벤트, 패킷을 만든다."""
    cfg = common.params()
    prepared = {}
    for task_id in sorted({task_id for _, task_id in sessions}):
        task = tasks.make_task(cfg["seed"], int(task_id[1:]))
        got = [sessions.get((p, task_id)) for p in common.PROVIDERS]
        if any(s is None or s["exit_code"] != 0 for s in got):
            continue
        shared = set.intersection(*(traversal(task, p, sessions[(p, task_id)]["rows"]) for p in common.PROVIDERS))
        facts = [fact for fact in task["facts"] if fact["id"] in shared]
        item_facts = {}
        for fact in facts:
            item_facts.setdefault(fact["item"], []).append(fact)
        entry = {"task": task, "facts": facts, "sources": {}}
        for source in common.PROVIDERS:
            events = replay(source, task_id, sessions[(source, task_id)]["rows"])
            records = view.build_records(events)
            tools = [(number, record) for number, record in enumerate(records, 2) if record["type"] == "tool"]
            evidence = set()
            for item_id in item_facts:
                markers = [fact["marker"] for fact in item_facts[item_id]]
                hit = next((n for n, r in tools if any(m in (r["output"] or "") for m in markers)), None)
                if hit:
                    evidence.add(hit)
            packet = make_packet(source, task, facts, records, evidence, budget)
            entry["sources"][source] = {"events": events, "packet": packet, "evidence_seqs": sorted(evidence)}
        prepared[task_id] = entry
    return prepared


def gate(prepared):
    cfg = common.params()
    total = sum(len(e["facts"]) for e in prepared.values())
    mean = total / len(prepared) if prepared else 0.0
    ok = mean >= cfg["gate_min_facts_per_task"] and total >= cfg["gate_min_facts_total"] and len(prepared) >= cfg["gate_min_tasks"]
    return ok, {"tasks": len(prepared), "common_facts": total, "mean_per_task": mean}


def receiver_answer(receiver, prompt, cwd):
    started = time.monotonic()
    try:
        result = subprocess.run(RECEIVER_COMMAND[receiver], input=prompt, capture_output=True, text=True, cwd=cwd,
                                timeout=common.params()["session_timeout_s"])
        answer = {"exit_code": result.returncode, "stdout": result.stdout}
    except subprocess.TimeoutExpired:
        answer = {"exit_code": None, "stdout": ""}
    answer["elapsed_s"] = round(time.monotonic() - started, 3)
    return answer


def snapshot(label):
    now = usage.snapshot()
    return {"label": label, "ts_utc": now_utc(), "claude_session_percent": now["claude"]["session"],
            "claude_week_percent": now["claude"]["week"], "codex_week_percent": now["codex"]["week"]}


def codex_refresh():
    """`--ephemeral` 세션은 사용률 기록을 남기지 않으므로 사용률을 읽기 전에 짧은 세션 하나를 남긴다."""
    subprocess.run(["codex", "exec", "-m", common.MODELS["codex"], "--ignore-user-config", "--ignore-rules",
                    "--skip-git-repo-check", "--sandbox", "read-only", "-"], input="OK라고만 답하라", capture_output=True,
                   text=True, check=False, cwd=common.ORCH, timeout=120)


def load_env():
    with open(os.path.join(common.EXP, "env.json"), encoding="utf-8") as handle:
        return json.load(handle)


def save_env(env):
    with open(os.path.join(common.EXP, "env.json"), "w", encoding="utf-8") as handle:
        json.dump(env, handle, ensure_ascii=False, indent=2)
        handle.write("\n")


def record_usage(label, start=False):
    codex_refresh()
    snap = snapshot(label)
    env = load_env()
    env["usage"]["snapshots"].append(snap)
    if start:
        env["usage"]["start"] = snap
    save_env(env)
    return snap


def guard_usage(label):
    """두 실험 합산 한도(#218 시작 대비 +5%p)와 이 실험의 시작 대비 한도를 확인하고, 5시간 창이 85%를 넘으면 기다린다."""
    cfg = common.params()
    snap = record_usage(label)
    base = cfg["combined_base"]
    for key, name in (("claude_week_percent", "claude"), ("codex_week_percent", "codex")):
        if snap[key] is None:
            fail(f"사용률을 읽지 못했다: {key}", 3)
        if snap[key] - base[name] >= cfg["combined_limit_pp"]:
            fail(f"{key}가 합산 시작 {base[name]}%에서 {snap[key]}%로 {cfg['combined_limit_pp']}%p에 닿아 멈췄다", 3)
    waited = 0
    while snap["claude_session_percent"] is not None and snap["claude_session_percent"] > cfg["session_wait_percent"]:
        print(f"01-collect: Claude 5시간 창 {snap['claude_session_percent']}%, 초기화를 기다린다", file=sys.stderr)
        time.sleep(600)
        waited += 1
        snap = {**snap, "claude_session_percent": usage.claude()["session"]}
        if waited > 40:
            fail("Claude 5시간 창이 초기화되지 않아 멈췄다", 3)


def jsonl(path, rows):
    with open(path, "a", encoding="utf-8", newline="\n") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def read_jsonl(path):
    if not os.path.exists(path):
        return []
    return [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()]


def run_lane(receiver, items, rid, raw_dir, done, state, lock):
    cwd = os.path.join(common.ORCH, "receiver-work")
    os.makedirs(cwd, exist_ok=True)
    path = os.path.join(raw_dir, f"receiver-{receiver}-{rid}.jsonl")
    for task_id, source, prompt in items:
        trial = f"{source}-{receiver}-{task_id}"
        if trial in done:
            continue
        for attempt in (1, 2):
            answer = receiver_answer(receiver, prompt, cwd)
            if answer["exit_code"] == 0:
                break
        head = {"run_id": rid, "trial_id": trial, "condition": f"{source}->{receiver}", "ts_utc": now_utc(), "task_id": task_id}
        with lock:
            jsonl(path, [{**head, "dir": "in", "line": prompt}, {**head, "dir": "out", "line": json.dumps({**answer, "attempt": attempt}, ensure_ascii=False)}])
            state["failures"] = 0 if answer["exit_code"] == 0 else state["failures"] + 1
            if state["failures"] >= MAX_CONSECUTIVE_FAILURES:
                state["stop"] = True
        if state["stop"]:
            return


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--budget", type=int, default=None)
    parser.add_argument("--limit-tasks", type=int, default=None)
    parser.add_argument("--resume")
    args = parser.parse_args()
    cfg = common.params()
    budget = args.budget or cfg["packet_budget_tokens"]
    os.makedirs(common.ORCH, exist_ok=True)
    common.build_examples()
    prepared = prepare(load_sessions(), budget)
    ok, summary = gate(prepared)
    print("gate", "run" if ok else "stop", json.dumps(summary), flush=True)
    if args.dry_run:
        rows = []
        for task_id, entry in sorted(prepared.items()):
            for source, data in entry["sources"].items():
                packet = data["packet"]
                hit = sum(1 for fact in entry["facts"] if fact["marker"] in packet["packet"])
                rows.append({"task": task_id, "source": source, "budget": budget, "tokens": packet["tokens"],
                             "included": len(packet["included"]), "facts": len(entry["facts"]), "facts_in_packet": hit})
        with open(os.path.join(common.ORCH, f"dry-run-{budget}.json"), "w", encoding="utf-8") as handle:
            json.dump({"gate": summary, "rows": rows}, handle, indent=1)
        for source in common.PROVIDERS:
            sub = [r for r in rows if r["source"] == source]
            print(source, "budget", budget, "facts in packet", sum(r["facts_in_packet"] for r in sub), "/", sum(r["facts"] for r in sub),
                  "mean tokens", sum(r["tokens"] for r in sub) / len(sub))
        return
    for tool in ("claude", "codex"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            fail(f"{tool}이 PATH에 없다")
    if not ok:
        fail("근거 사실이 모자라 받는 쪽을 돌리지 않는다", 4)
    raw_dir = os.path.join(common.EXP, "data", "raw")
    os.makedirs(raw_dir, exist_ok=True)
    commit = subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], cwd=common.REPO, capture_output=True, text=True).stdout.strip()
    rid = args.resume or f"{datetime.datetime.now(datetime.timezone.utc):%Y%m%dT%H%M%SZ}-{commit}"
    if not args.resume:
        stamp = now_utc()
        for source in common.PROVIDERS:
            jsonl(os.path.join(raw_dir, f"events-{source}-{rid}.jsonl"), (
                {"run_id": rid, "trial_id": f"{source}-{task_id}", "condition": source, "ts_utc": stamp, "task_id": task_id,
                 "dir": "event", "line": json.dumps(event, ensure_ascii=False)}
                for task_id, entry in sorted(prepared.items()) for event in entry["sources"][source]["events"]))
        jsonl(os.path.join(raw_dir, f"packets-{rid}.jsonl"), (
            {"run_id": rid, "trial_id": f"{source}-{task_id}", "condition": source, "ts_utc": stamp, "task_id": task_id, "dir": "packet",
             "line": json.dumps({"packet": data["packet"]["packet"], "tokens": data["packet"]["tokens"],
                                 "included": data["packet"]["included"], "evidence_seqs": data["evidence_seqs"],
                                 "facts": [f["id"] for f in entry["facts"]], "budget": budget}, ensure_ascii=False)}
            for task_id, entry in sorted(prepared.items()) for source, data in entry["sources"].items()))
        env = load_env()
        env["run_id"], env["commit"] = rid, commit
        env["run_date"] = datetime.date.today().isoformat()
        env["tools"].update({"codex": subprocess.run(["codex", "--version"], capture_output=True, text=True).stdout.strip(),
                             "claude": subprocess.run(["claude", "--version"], capture_output=True, text=True).stdout.strip(),
                             "python": sys.version.split()[0]})
        save_env(env)
        record_usage("start", start=True)
    order = [(t, s, r) for t in sorted(prepared) for s in common.PROVIDERS for r in common.PROVIDERS]
    random.Random(cfg["seed"]).shuffle(order)
    task_order = []
    for t, _, _ in order:
        if t not in task_order:
            task_order.append(t)
    task_order = task_order[:args.limit_tasks] if args.limit_tasks else task_order
    done = {r["trial_id"] for p in glob.glob(os.path.join(raw_dir, f"receiver-*-{rid}.jsonl")) for r in read_jsonl(p) if r["dir"] == "out"}
    lock, state = threading.Lock(), {"failures": 0, "stop": False}
    for first in range(0, len(task_order), BATCH_TASKS):
        chunk = set(task_order[first:first + BATCH_TASKS])
        lanes = {r: [] for r in common.PROVIDERS}
        for task_id, source, receiver in order:
            if task_id not in chunk:
                continue
            entry = prepared[task_id]
            prompt = RECEIVER_PROMPT.format(packet=entry["sources"][source]["packet"]["packet"],
                                            questions="\n".join(f"{fact['id']}: {fact['question']}" for fact in entry["facts"]))
            lanes[receiver].append((task_id, source, prompt))
        threads = [threading.Thread(target=run_lane, args=(r, items, rid, raw_dir, done, state, lock)) for r, items in lanes.items()]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        if state["stop"]:
            fail("연속 5개 세션이 실패해 중단했다. 원인을 고친 뒤 새 실행 id로 다시 수집한다")
        guard_usage(f"after-batch-{first // BATCH_TASKS + 1}")
        print(f"01-collect: 작업 {min(first + BATCH_TASKS, len(task_order))}/{len(task_order)} 완료", flush=True)
    record_usage("pilot-end" if args.limit_tasks and args.limit_tasks < len(prepared) else "end")
    print(rid)


if __name__ == "__main__":
    main()
