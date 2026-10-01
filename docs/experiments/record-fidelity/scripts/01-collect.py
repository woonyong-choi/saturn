#!/usr/bin/env python3
"""1단계 수집: 합성 작업마다 Codex와 Claude Code가 같은 지시를 실제로 진행하고, 오간 원시 줄을 저장한다.

사용: 01-collect.py stage1 [--pilot] [--tasks N] [--providers codex,claude]
      01-collect.py stage2 [--dry-run]
  --pilot    작업 3개(시드 1990)를 오케스트레이션 폴더에만 저장하고 저장소에는 쓰지 않는다.
  --dry-run  2단계 받는 쪽 지시문만 만들어 첫 건을 보이고 provider를 부르지 않는다.
동시 실행은 provider당 하나, 합쳐 둘 이하다.
"""
import argparse
import datetime
import getpass
import importlib
import json
import os
import random
import shutil
import socket
import subprocess
import sys
import threading

import common
import tasks
import usage

LIMITS = {"claude_week_delta": 3.0, "codex_week_delta": 3.0, "claude_session": 85.0}


def run_id():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    commit = subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], cwd=common.REPO, capture_output=True, text=True).stdout.strip()
    return f"{stamp}-{commit}"


def materialize(task, workdir):
    shutil.rmtree(workdir, ignore_errors=True)
    for rel, text in task["files"].items():
        path = os.path.join(workdir, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(text)
    subprocess.run(["git", "init", "-q"], cwd=workdir, check=True)


def redactions(workdir):
    return [
        [workdir, "/work"],
        [os.path.expanduser("~"), "/home/user"],
        [socket.gethostname(), "host"],
        [getpass.getuser(), "user"],
    ]


def run_session(provider, task, root):
    name = f"{provider}-{task['task_id']}"
    workdir = os.path.join(root, "work", name)
    materialize(task, workdir)
    prompt = os.path.join(root, "prompts", f"{task['task_id']}.txt")
    os.makedirs(os.path.dirname(prompt), exist_ok=True)
    with open(prompt, "w", encoding="utf-8") as handle:
        handle.write(task["instruction"])
    raw = os.path.join(root, "raw", f"{name}.jsonl")
    live = os.path.join(root, "live", f"{name}.jsonl")
    for path in (raw, live):
        os.makedirs(os.path.dirname(path), exist_ok=True)
    env = dict(os.environ)
    env.update({
        "SHIM_REAL": provider,
        "SHIM_RAW": raw,
        "SHIM_EXTRA": json.dumps(common.EXTRA[provider]),
        "SHIM_REDACT": json.dumps(redactions(workdir)),
    })
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    result = subprocess.run(
        [common.EXAMPLE, provider, "--program", common.SHIM, "--workdir", workdir,
         "--prompt-file", prompt, "--out", live, "--model", common.MODELS[provider],
         "--timeout-s", str(common.params()["session_timeout_s"])],
        env=env, capture_output=True, text=True,
    )
    return {"provider": provider, "task_id": task["task_id"], "started": started,
            "exit_code": result.returncode, "stderr_tail": result.stderr[-300:], "raw": raw, "live": live}


def worker(provider, task_list, root, outcomes):
    for task in task_list:
        outcomes.append(run_session(provider, task, root))


def check_usage(start, limits):
    now = usage.snapshot()
    claude_delta = now["claude"]["week"] - start["claude"]["week"]
    codex_delta = now["codex"]["week"] - start["codex"]["week"]
    stop = (claude_delta > limits["claude_week_delta"] or codex_delta > limits["codex_week_delta"])
    return now, stop


def merge(root, rid, out_dir, outcomes):
    os.makedirs(out_dir, exist_ok=True)
    for provider in common.PROVIDERS:
        path = os.path.join(out_dir, f"{provider}-{rid}.jsonl")
        with open(path, "w", encoding="utf-8") as sink:
            for item in sorted((o for o in outcomes if o["provider"] == provider), key=lambda o: o["task_id"]):
                head = {"run_id": rid, "trial_id": f"{provider}-{item['task_id']}", "condition": provider,
                        "ts_utc": item["started"], "task_id": item["task_id"]}
                sink.write(json.dumps({**head, "dir": "meta", "line": json.dumps({"exit_code": item["exit_code"]})}) + "\n")
                with open(item["raw"], encoding="utf-8") as source:
                    for line in source:
                        row = json.loads(line)
                        sink.write(json.dumps({**head, **row}, ensure_ascii=False) + "\n")


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


def receiver_answer(receiver, prompt, cwd):
    result = subprocess.run(RECEIVER_COMMAND[receiver], input=prompt, capture_output=True, text=True, cwd=cwd, timeout=common.params()["session_timeout_s"])
    return {"exit_code": result.returncode, "stdout": result.stdout}


def stage2(dry_run):
    """작업마다 두 기록의 oracle 패킷을 두 받는 쪽 새 session에 넘기고, 근거 사실 질문의 답을 저장한다."""
    gate = os.path.join(common.EXP, "results", "stage1-gate.json")
    if not os.path.exists(gate) or json.load(open(gate))["stage2"] != "run":
        sys.exit("1단계 판정이 2단계 진행이 아니다. 2단계를 하지 않는다")
    pipeline = importlib.import_module("02-process")
    cfg = common.params()
    sessions = pipeline.load_sessions("data")
    rid = run_id()
    cwd = os.path.join(common.ORCH, "receiver-work")
    os.makedirs(cwd, exist_ok=True)
    rows_out = {receiver: [] for receiver in common.PROVIDERS}
    order = [(t, s, r) for t in sorted({k[1] for k in sessions}) for s in common.PROVIDERS for r in common.PROVIDERS]
    random.Random(cfg["seed"]).shuffle(order)
    for task_id, source, receiver in order:
        task = tasks.make_task(cfg["seed"], int(task_id[1:]))
        both = [sessions.get((p, task_id)) for p in common.PROVIDERS]
        if any(s is None or s["exit_code"] != 0 for s in both):
            continue
        item_facts = {}
        for fact in task["facts"]:
            item_facts.setdefault(fact["item"], []).append(fact)
        shared = set.intersection(*(pipeline.traversal(task, p, sessions[(p, task_id)]["rows"])[0] for p in common.PROVIDERS))
        facts = [fact for fact in task["facts"] if fact["id"] in shared]
        records, _, evidence = pipeline.saturn_records(task, source, sessions[(source, task_id)]["rows"], item_facts)
        packet = pipeline.make_packet(source, task_id, task, records, evidence, cfg["packet_budget_tokens"], "judge-only")["packet"]
        prompt = RECEIVER_PROMPT.format(packet=packet, questions="\n".join(f"{fact['id']}: {fact['question']}" for fact in facts))
        if dry_run:
            print(task_id, source, receiver, len(facts), len(prompt))
            print(prompt)
            return
        answer = receiver_answer(receiver, prompt, cwd)
        head = {"run_id": rid, "trial_id": f"{source}-{receiver}-{task_id}", "condition": f"{source}->{receiver}",
                "ts_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "task_id": task_id}
        rows_out[receiver].append({**head, "dir": "in", "line": prompt})
        rows_out[receiver].append({**head, "dir": "out", "line": json.dumps(answer, ensure_ascii=False)})
    out_dir = os.path.join(common.EXP, "data", "raw")
    for receiver, rows in rows_out.items():
        with open(os.path.join(out_dir, f"receiver-{receiver}-{rid}.jsonl"), "w", encoding="utf-8") as handle:
            for row in rows:
                handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("stage")
    parser.add_argument("--pilot", action="store_true")
    parser.add_argument("--tasks", type=int, default=None)
    parser.add_argument("--providers", default="codex,claude")
    args = parser.parse_args()
    if args.stage == "stage2":
        return stage2(args.dry_run)
    if args.stage != "stage1":
        sys.exit("01-collect.py: stage1과 stage2만 지원한다")
    cfg = common.params()
    seed, count = (1990, 3) if args.pilot else (cfg["seed"], args.tasks or cfg["tasks"])
    providers = args.providers.split(",")
    task_list = [tasks.make_task(seed, number) for number in range(1, count + 1)]
    common.build_example()
    rid = run_id()
    root = os.path.join(common.ORCH, "pilot" if args.pilot else f"runs/{rid}")
    start = usage.snapshot()
    print("usage start", json.dumps(start), flush=True)
    if start["claude"]["session"] is not None and start["claude"]["session"] > LIMITS["claude_session"]:
        sys.exit("claude 5시간 창이 85%를 넘었다. 초기화 뒤 다시 실행한다")
    outcomes = []
    batch = 4
    for first in range(0, len(task_list), batch):
        chunk = task_list[first:first + batch]
        threads = [threading.Thread(target=worker, args=(p, chunk, root, outcomes)) for p in providers]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        now, stop = check_usage(start, LIMITS)
        print(f"batch {first // batch + 1} usage", json.dumps(now), "failed", [o["exit_code"] for o in outcomes if o["exit_code"]], flush=True)
        if stop:
            print("사용률 한도에 닿아 멈춘다", flush=True)
            break
    if not args.pilot:
        merge(root, rid, os.path.join(common.EXP, "data", "raw"), outcomes)
    with open(os.path.join(root, "outcomes.json"), "w", encoding="utf-8") as handle:
        json.dump({"run_id": rid, "start": start, "end": usage.snapshot(), "outcomes": outcomes}, handle, ensure_ascii=False, indent=1)
    print("done", rid)


if __name__ == "__main__":
    main()
