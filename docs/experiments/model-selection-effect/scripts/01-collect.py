#!/usr/bin/env python3
"""설계에 고정한 과제와 팔을 무작위 순서로 실제 saturn에 보내고 trial마다 원자료 하나를 남긴다.

사용: 01-collect.py dev|confirm [--workers N] [--limit N]
설계(design.md)와 이 실행기를 커밋한 뒤에만 시작한다. 자동 재시도는 없다.
"""
from __future__ import annotations

import argparse
import gzip
import json
import random
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import engine_run  # noqa: E402
import tasks  # noqa: E402

DATA = Path(__file__).resolve().parents[1] / "data"
PHASES = {
    # 상한은 design.md에 적은 값이다.
    "dev": dict(first_tid=0, shadow=True, seed=542100, provider_cap=110, router_cap=110),
    "confirm": dict(first_tid=100, shadow=False, seed=542200, provider_cap=300, router_cap=300),
}
STOP_AFTER_FAILURES = 6


def schedule(phase: str) -> list[tuple[int, dict, str]]:
    plan = PHASES[phase]
    items = [(task, arm) for task in tasks.make_tasks(phase) for arm in engine_run.ARMS]
    random.Random(plan["seed"]).shuffle(items)
    return [(plan["first_tid"] + i, task, arm) for i, (task, arm) in enumerate(items)]


def raw_path(phase: str, tid: int, task: dict, arm: str) -> Path:
    return DATA / "raw" / phase / f"{tid:03d}-{task['task_id']}-{arm}.json.gz"


def counts(result: dict) -> tuple[int, int]:
    t = result["tables"]
    runs = len(t["runs"]) if isinstance(t.get("runs"), list) else 0
    judgments = len(t["judgments"]) if isinstance(t.get("judgments"), list) else 0
    return runs, judgments


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("phase", choices=PHASES)
    parser.add_argument("--workers", type=int, default=3)
    parser.add_argument("--limit", type=int, default=0)
    args = parser.parse_args()
    plan = PHASES[args.phase]
    key = engine_run.router_key()
    items = schedule(args.phase)
    pending = [it for it in items if not raw_path(args.phase, it[0], it[1], it[2]).exists()]
    if args.limit:
        pending = pending[: args.limit]
    (DATA / "raw" / args.phase).mkdir(parents=True, exist_ok=True)
    ledger = DATA / f"ledger-{args.phase}.jsonl"
    lock = threading.Lock()
    state = dict(provider=0, router=0, failures=0, stop=False)
    for tid, task, arm in items:
        path = raw_path(args.phase, tid, task, arm)
        if path.exists():
            with gzip.open(path, "rt", encoding="utf-8") as f:
                runs, judgments = counts(json.load(f))
            state["provider"] += runs
            state["router"] += judgments

    def one(item):
        tid, task, arm = item
        with lock:
            if state["stop"] or state["provider"] >= plan["provider_cap"] or state["router"] >= plan["router_cap"]:
                state["stop"] = True
                return
            # 이 시도가 부를 수 있는 호출을 먼저 예약한다
            state["provider"] += 1
            state["router"] += 1
        try:
            result = engine_run.run_trial(args.phase, tid, task, arm, plan["shadow"], key)
        except Exception as error:  # noqa: BLE001
            with lock:
                state["failures"] += 1
                state["stop"] = state["failures"] >= STOP_AFTER_FAILURES
                print(f"trial {tid} {task['task_id']} {arm} crashed: {type(error).__name__}", flush=True)
            return
        runs, judgments = counts(result)
        with lock:
            state["provider"] += runs - 1
            state["router"] += judgments - 1
            state["failures"] = state["failures"] + 1 if result["status"] == "failed" else 0
            if state["failures"] >= STOP_AFTER_FAILURES:
                state["stop"] = True
            raw_path(args.phase, tid, task, arm).write_bytes(gzip.compress(json.dumps(result, ensure_ascii=False, sort_keys=True).encode(), mtime=0))
            with ledger.open("a", encoding="utf-8") as f:
                f.write(json.dumps(dict(tid=tid, task_id=task["task_id"], arm=arm, status=result["status"], success=result["check"]["success"],
                                        provider_requests=runs, router_calls=judgments, at=time.time()), ensure_ascii=False) + "\n")
            print(f"{tid:03d} {task['task_id']:12s} {arm:9s} {result['status']:9s} ok={result['check']['success']} totals p={state['provider']} r={state['router']}", flush=True)

    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        list(pool.map(one, pending))
    print(f"done: provider={state['provider']} router={state['router']} stopped={state['stop']}", flush=True)


if __name__ == "__main__":
    main()
