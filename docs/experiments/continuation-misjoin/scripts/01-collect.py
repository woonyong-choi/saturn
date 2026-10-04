"""봉인한 조건을 순서대로 수집하고 응답을 단계별로 보존한다."""

from __future__ import annotations

import concurrent.futures
import http.client
import json
import os
import random
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

from support import (
    BASE,
    CONDITIONS,
    PRIVATE,
    PUBLIC,
    SEED,
    WORKTREE,
    append,
    load,
    now,
    read,
    reserve,
    rows,
    setup,
    sha,
    write,
)

judge = load("baseline_judge", "03-judge.py")
KEEP = (
    "Does this input continue, correct, complete, or verify the same concrete goal or deliverable as the previous task? "
    "Answer no for a separately completable goal or deliverable, even in the same repository, file, topic, or broader project, "
    "and even when it uses the previous result as reference or calls itself a follow-up. "
    "A new defect, a separate documentation deliverable after implementation, or a separate experiment after cleanup is a new task "
    "unless it was already an unfinished part of the previous goal. "
    "An answer to the previous question, progress request, correction, changed constraint, or verification of that same deliverable continues it. "
    "For mixed requests, judge the dominant goal; do not treat a short introductory follow-up as evidence that the independent main request continues."
)
NEW = (
    "Does the main request introduce a separately completable new goal or deliverable rather than completing, correcting, or verifying the previous task? "
    "Sharing a repository, file, topic, or using previous results as reference does not make goals identical. "
    "A separately scoped defect, documentation deliverable, or experiment is new unless already an unfinished part of the previous goal. "
    "A progress question, answer, correction, changed constraint, or verification of the same deliverable is not new. "
    "For mixed requests, judge the dominant goal."
)


def request_for(case: dict, condition: str) -> dict:
    body = judge.request_for(case, "B")
    if condition == "B1":
        body["questions"]["keep_current"]["instructions"] = KEEP
    elif condition == "B2":
        body["questions"]["is_new_task"] = {"type": "noul", "instructions": NEW}
    elif condition == "B3":
        context = {
            "goal_excerpt": case["goal_excerpt"][:500],
            "progress_excerpt": case["progress_excerpt"][-1500:],
            "input": case["previous_input"],
        }
        state = body["state"].split("\nprevious task context: ", 1)[0]
        body["state"] = (
            state
            + "\nprevious task context: "
            + json.dumps(context, ensure_ascii=False, separators=(",", ":"))
        )
    else:
        raise ValueError("unknown condition")
    return body


def prepare() -> None:
    if (PRIVATE / "run.json").exists():
        return
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=WORKTREE, text=True
    ).strip()
    if subprocess.check_output(
        ["git", "status", "--porcelain", "--", str(PUBLIC)], cwd=WORKTREE, text=True
    ).strip():
        raise RuntimeError("commit design and collector before collection")
    names = [
        "sample.json",
        "final-labels.json",
        "jev.jsonl",
        "run.json",
        "census.json",
        "calls.jsonl",
    ]
    write(
        PRIVATE / "run.json",
        {
            "run_id": now().replace(":", "").replace("-", "")[:15] + "-" + head[:7],
            "design_commit": head,
            "started": now(),
            "base_hashes": {n: sha(BASE / n) for n in names},
        },
    )
    extractor = load("baseline_extract", "01-collect.py")
    original = read(BASE / "sample.json")
    seen = {(c["project"], c["uuid"]) for c in original}
    ids = {c["id"] for c in original}
    counts, extra, files = Counter(), [], []
    for path in sorted((Path.home() / ".claude/projects").glob("*/*.jsonl")):
        cases, meta = extractor.extract(path, counts)
        files.append(meta)
        for case in cases:
            key = (case["project"], case["uuid"])
            if case["id"] in ids or (case["uuid"] and key in seen):
                continue
            seen.add(key)
            extra.append(case)
    rng = random.Random(SEED)
    rng.shuffle(extra)
    write(PRIVATE / "sample.json", extra[:58])
    write(
        PRIVATE / "extension-census.json",
        {
            "counts": dict(counts),
            "files": files,
            "eligible_extra": len(extra),
            "selected_extra": min(58, len(extra)),
        },
    )
    size = {
        condition: [
            len(
                json.dumps(
                    request_for(c, condition), ensure_ascii=False, separators=(",", ":")
                ).encode()
            )
            for c in original + extra[:58]
        ]
        for condition in CONDITIONS
    }
    write(
        PRIVATE / "preflight.json",
        {
            c: {"max": max(v), "oversize": sum(n > 100000 for n in v)}
            for c, v in size.items()
        },
    )
    print(
        json.dumps(
            {
                "base": len(original),
                "extension": min(58, len(extra)),
                "max_request_bytes": {c: max(v) for c, v in size.items()},
            }
        ),
        flush=True,
    )


# cost: io 1 HTTPS request per trial; basis: estimate
def post(case: dict, condition: str, repeat: int) -> dict:
    key = os.environ["SATURN_JUDGE_KEY"]
    body = request_for(case, condition)
    encoded = (
        json.dumps(body, ensure_ascii=False, separators=(",", ":"))
        .replace(key, "[secret]")
        .encode()
    )
    trial = f"{case['id']}-{condition}-r{repeat}"
    row = {
        "run_id": read(PRIVATE / "run.json")["run_id"],
        "trial_id": trial,
        "id": case["id"],
        "condition": condition,
        "repeat": repeat,
        "ts_utc": now(),
        "request": json.loads(encoded),
        "request_bytes": len(encoded),
        "http_status": None,
    }
    if len(encoded) > 100000:
        return {**row, "status": "oversize"}
    reserve("jev", trial)
    request = urllib.request.Request(
        judge.ENDPOINT,
        data=encoded,
        method="POST",
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    start = time.monotonic()
    try:
        with urllib.request.build_opener(judge.NoRedirect).open(
            request, timeout=45
        ) as response:
            text = response.read().decode().replace(key, "[secret]")
            row.update(http_status=response.status, raw_response=text)
            reply = json.loads(text)
            row.update(
                model=reply.get("model"),
                answers=judge.parse(reply, body["questions"]),
                status="ok",
            )
            if reply.get("model") != judge.MODEL:
                row["status"] = "model_mismatch"
    except urllib.error.HTTPError as error:
        row.update(
            status="http_error",
            http_status=error.code,
            raw_response=error.read().decode(errors="replace").replace(key, "[secret]"),
        )
    except (
        OSError,
        http.client.HTTPException,
        ValueError,
        TypeError,
        AttributeError,
    ) as error:
        row.update(status="failed", error=type(error).__name__)
    row["latency_ms"] = (time.monotonic() - start) * 1000
    return row


def collect() -> None:
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("missing judge key")
    sample = read(BASE / "sample.json") + read(PRIVATE / "sample.json")
    rng = random.Random(SEED)
    tasks = [
        (c, condition, repeat)
        for repeat in (1, 2)
        for c in sample
        for condition in rng.sample(list(CONDITIONS), 3)
    ]
    saved = {r["trial_id"] for r in rows(PRIVATE / "jev.jsonl")}
    reserved = {
        r["trial_id"] for r in rows(PRIVATE / "calls.jsonl") if r["kind"] == "jev"
    }
    pending = []
    for c, condition, repeat in tasks:
        trial = f"{c['id']}-{condition}-r{repeat}"
        if trial in saved:
            continue
        if trial in reserved:
            append(
                PRIVATE / "jev.jsonl",
                {
                    "trial_id": trial,
                    "id": c["id"],
                    "condition": condition,
                    "repeat": repeat,
                    "status": "incomplete",
                    "run_id": read(PRIVATE / "run.json")["run_id"],
                    "ts_utc": now(),
                    "request_bytes": None,
                },
            )
        else:
            pending.append((c, condition, repeat))
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        for start in range(0, len(pending), 6):
            futures = [pool.submit(post, *task) for task in pending[start : start + 6]]
            stop = False
            for future in concurrent.futures.as_completed(futures):
                row = future.result()
                append(PRIVATE / "jev.jsonl", row)
                text = row.get("raw_response", "").lower()
                stop |= (
                    row.get("http_status") in (401, 403)
                    or row["status"] == "model_mismatch"
                    or (
                        "model" in text
                        and any(
                            s in text
                            for s in (
                                "not found",
                                "unsupported",
                                "not available",
                                "not supported",
                            )
                        )
                    )
                )
            if stop:
                raise RuntimeError("judge authentication or model rejected")
            if start % 120 == 0:
                print(
                    json.dumps(
                        {
                            "completed": len(saved) + start + len(futures),
                            "scheduled": len(tasks),
                        }
                    ),
                    flush=True,
                )


def label(lane: str) -> None:
    module = load("baseline_label", "02-label.py")
    module.PRIVATE = PRIVATE
    module.reserve = reserve
    module.setup = setup
    sys.argv = [sys.argv[0], lane]
    module.main()


def main() -> None:
    setup()
    mode = sys.argv[1]
    if mode == "prepare":
        prepare()
    elif mode == "judge":
        collect()
    elif mode == "label":
        label(sys.argv[2])
    else:
        raise ValueError("unknown collection phase")


if __name__ == "__main__":
    main()
