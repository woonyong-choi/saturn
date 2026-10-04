"""독립 라벨 묶음과 두 조건 호출을 상한 안에서 실행한다."""

from __future__ import annotations

import concurrent.futures
import json
import os
import random
import sys

from runtime import (
    MODELS,
    PRIVATE,
    SEED,
    append,
    collector,
    labeler,
    read,
    rows,
    setup,
    write,
)


# cost: io up to 2*ceil(n/15) CLI calls per lane, capped globally; basis: estimate
def label(lane: str) -> None:
    cases = read(PRIVATE / "sample.json")
    prior = {}
    if lane == "adjudicated":
        indices = [
            {r["id"]: r for r in read(PRIVATE / f"labels-{m}.json")} for m in MODELS
        ]
        prior = {
            c["id"]: [
                idx.get(c["id"], {"label": "uncertain", "reason": "missing"})
                for idx in indices
            ]
            for c in cases
        }
        cases = [
            c
            for c in cases
            if prior[c["id"]][0]["label"] != prior[c["id"]][1]["label"]
            or prior[c["id"]][0]["label"] == "uncertain"
        ]
    chunks = [(i // 15, cases[i : i + 15]) for i in range(0, len(cases), 15)]
    results = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for start in range(0, len(chunks), 4):
            futures = {
                pool.submit(labeler.label_batch, i, chunk, lane, prior): i
                for i, chunk in chunks[start : start + 4]
            }
            for future in concurrent.futures.as_completed(futures):
                results[futures[future]] = future.result()
                print(
                    json.dumps(
                        {
                            "lane": lane,
                            "batches_done": len(results),
                            "batches_total": len(chunks),
                        }
                    ),
                    flush=True,
                )
    write(
        PRIVATE / f"labels-{lane}.json",
        [r for i in sorted(results) for r in results[i]],
    )


# cost: io up to 2*n HTTPS calls, capped at 4000; vars: n = cases; basis: estimate
def judge() -> None:
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("missing judge key")
    cases = read(PRIVATE / "sample.json")
    rng = random.Random(SEED)
    tasks = [(c, b, 1) for c in cases for b in rng.sample(["B1", "B2"], 2)]
    saved = {r["trial_id"] for r in rows(PRIVATE / "jev.jsonl")}
    reserved = {
        r["trial_id"] for r in rows(PRIVATE / "calls.jsonl") if r["kind"] == "jev"
    }
    pending = []
    for c, b, repeat in tasks:
        trial = f"{c['id']}-{b}-r1"
        if trial in saved:
            continue
        if trial in reserved:
            append(
                PRIVATE / "jev.jsonl",
                {
                    "trial_id": trial,
                    "id": c["id"],
                    "condition": b,
                    "repeat": repeat,
                    "status": "incomplete",
                    "run_id": read(PRIVATE / "run.json")["run_id"],
                    "ts_utc": collector.now(),
                    "request_bytes": None,
                },
            )
        else:
            pending.append((c, b, repeat))
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        for start in range(0, len(pending), 6):
            futures = [
                pool.submit(collector.post, *task)
                for task in pending[start : start + 6]
            ]
            stop = False
            for future in concurrent.futures.as_completed(futures):
                row = future.result()
                append(PRIVATE / "jev.jsonl", row)
                stop |= (
                    row.get("http_status") in (401, 403)
                    or row["status"] == "model_mismatch"
                )
            if stop:
                raise RuntimeError("judge authentication or model rejected")
            if start % 120 == 0:
                print(
                    json.dumps(
                        {
                            "judge_done": len(saved) + start + len(futures),
                            "judge_total": len(tasks),
                        }
                    ),
                    flush=True,
                )


if __name__ == "__main__":
    setup()
    if sys.argv[1] == "judge":
        judge()
    elif sys.argv[1] in (*MODELS, "adjudicated"):
        label(sys.argv[1])
    else:
        raise ValueError("unknown collection phase")
