"""같은 입력을 세 번 판단하고 단계별 응답과 지연을 보존한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import random
import threading
import time

from protocol import (
    CONT_SCHEMA,
    GUIDE,
    KEEP,
    MODEL,
    REQUEST,
    SCHEMA,
    SEED,
    questions,
    scope_candidates,
    state,
    validate,
)
from runtime import PRIVATE, STOP, cli, jev, now, read, setup, write

CLAUDE_SLOTS = threading.Semaphore(4)


def continuation_body(item: dict) -> dict:
    activity = "running" if item["running"] else "idle"
    s = f"chat: {activity}\nprevious input handled as: queue\nuser input: {item['input']}"
    context = {
        k: item[v]
        for k, v in [
            ("input", "previous_input"),
            ("goal_excerpt", "goal_excerpt"),
            ("progress_excerpt", "progress_excerpt"),
        ]
    }
    s += "\nprevious task context: " + json.dumps(
        context, ensure_ascii=False, separators=(",", ":")
    )
    q = {
        "keep_current": {"type": "noul", "instructions": KEEP},
        "is_actionable": {
            "type": "noul",
            "instructions": "Is this input clear enough to act on without exploring the codebase first?",
        },
    }
    if item["running"]:
        for name, text, options in [
            (
                "relation_to_running",
                "How does this input relate to the work that is running now?",
                ["refines", "continues", "independent", "conflicts", "other"],
            ),
            (
                "steer_or_spawn",
                "Should this input be added to the running turn (steer), wait until the turn ends (queue), or start a separate agent (spawn)?",
                ["steer", "queue", "spawn", "other"],
            ),
        ]:
            q[name] = {
                "type": "choice",
                "instructions": text,
                "criteria": dict.fromkeys(options),
            }
    return {"model": MODEL, "state": s, "questions": q}


def maximum(values: dict) -> str:
    return max(values, key=values.get)


def decode(item: dict, answers: dict, request_p: float) -> dict:
    if request_p < 0.5:
        return dict(request="none", target="none", kind="none", scope_text=None)
    target = maximum(
        {k: v for k, v in answers["release_target"].items() if k != "none"}
    )
    kind = maximum(answers["kind"])
    scope = scope_candidates(item["text"])[maximum(answers["scope"])]
    return dict(
        request="release" if kind == "permanent" else "exception",
        target=target,
        kind=kind,
        scope_text=None if kind == "permanent" else scope,
    )


def run_jev(item: dict, condition: str, trial: str) -> dict:
    if item["task"] == "continuation":
        receipt = jev(continuation_body(item), trial)
        prediction = (
            {"continue": receipt["probabilities"]["keep_current"] >= 0.5}
            if receipt["status"] == "ok"
            else None
        )
        return dict(prediction=prediction, status=receipt["status"], stages=[trial])
    s, q = state(item), questions(item)
    if condition == "J1":
        receipt = jev(dict(model=MODEL, state=s, questions=q), trial)
        if receipt["status"] != "ok":
            return dict(prediction=None, status=receipt["status"], stages=[trial])
        answers = receipt["probabilities"]
        request_p = 1 - answers["release_target"]["none"]
        return dict(
            prediction=decode(item, answers, request_p),
            probabilities=answers,
            request_p=request_p,
            status="ok",
            stages=[trial],
        )
    stages = [trial + "-A"]
    a = jev(
        dict(
            model=MODEL,
            state=s,
            questions={"is_request": {"type": "noul", "instructions": REQUEST}},
        ),
        stages[-1],
    )
    if a["status"] != "ok":
        return dict(prediction=None, status=a["status"], stages=stages)
    p = a["probabilities"]["is_request"]
    if p < 0.5:
        return dict(
            prediction=dict(
                request="none", target="none", kind="none", scope_text=None
            ),
            request_p=p,
            status="ok",
            stages=stages,
        )
    s["stage_A"] = {"is_request": True}
    stages.append(trial + "-B")
    b = jev(
        dict(model=MODEL, state=s, questions={"release_target": q["release_target"]}),
        stages[-1],
    )
    if b["status"] != "ok":
        return dict(prediction=None, status=b["status"], stages=stages)
    if maximum(b["probabilities"]["release_target"]) == "none":
        return dict(
            prediction=dict(
                request="none", target="none", kind="none", scope_text=None
            ),
            request_p=p,
            status="ok",
            stages=stages,
        )
    s["stage_B"] = {"target": maximum(b["probabilities"]["release_target"])}
    stages.append(trial + "-C")
    c = jev(
        dict(model=MODEL, state=s, questions={k: q[k] for k in ("kind", "scope")}),
        stages[-1],
    )
    if c["status"] != "ok":
        return dict(prediction=None, status=c["status"], stages=stages)
    return dict(
        prediction=decode(item, {**b["probabilities"], **c["probabilities"]}, p),
        request_p=p,
        status="ok",
        stages=stages,
    )


# cost: io 1 official CLI call; basis: estimate
def run_claude(item: dict, trial: str) -> dict:
    is_cont = item["task"] == "continuation"
    s = continuation_body(item)["state"] if is_cont else state(item)
    schema = CONT_SCHEMA if is_cont else SCHEMA
    instructions = KEEP if is_cont else GUIDE
    prompt = (
        instructions
        + "\nOutput schema:\n"
        + json.dumps(schema, ensure_ascii=False)
        + "\nData:\n"
        + json.dumps(s, ensure_ascii=False)
    )
    with CLAUDE_SLOTS:
        receipt = cli("claude", "haiku", prompt, trial, schema)
    result = dict(prediction=None, status="failed", stages=[trial])
    try:
        wrapper = json.loads(receipt.get("stdout", ""))
        if not isinstance(wrapper, dict):
            raise ValueError("cli result must be an object")
        result.update(
            usage=wrapper.get("usage"),
            model_usage=wrapper.get("modelUsage"),
            total_cost_usd=wrapper.get("total_cost_usd"),
            num_turns=wrapper.get("num_turns"),
            session_id=wrapper.get("session_id"),
        )
        models = list(wrapper.get("modelUsage", {}))
        if not models or any(
            model not in ("claude-haiku-4-5-20251001", "claude-haiku-4-5")
            for model in models
        ):
            STOP.set()
            raise RuntimeError("unexpected or unreported claude model")
        if receipt["returncode"] != 0 or wrapper.get("is_error"):
            if any(
                term in (receipt.get("stdout", "") + receipt.get("stderr", "")).lower()
                for term in (
                    "authentication",
                    "unauthorized",
                    "not logged",
                    "invalid api key",
                    "model not found",
                    "not available",
                    "not supported",
                )
            ):
                STOP.set()
                raise RuntimeError("claude authentication or model rejected")
            return {**result, "error_subtype": wrapper.get("subtype")}
        prediction = json.loads(wrapper["result"])
        result.update(prediction=prediction, status="ok")
    except (ValueError, KeyError, TypeError):
        result["status"] = "invalid"
    return result


# cost: io up to 3 model calls per workflow; basis: estimate
def trial(task: tuple) -> dict:
    if STOP.is_set():
        raise RuntimeError("collection stopped")
    item, condition, repeat = task
    trial_id = f"{item['id']}-{condition}-r{repeat}"
    path = PRIVATE / "workflows" / (trial_id + ".json")
    if path.exists():
        return read(path)
    started = time.monotonic()
    row = (
        run_claude(item, trial_id)
        if condition == "L1"
        else run_jev(item, condition, trial_id)
    )
    row.update(
        run_id=read(PRIVATE / "run.json")["run_id"],
        trial_id=trial_id,
        item_id=item["id"],
        task=item["task"],
        condition=condition,
        repeat=repeat,
        ts_utc=now(),
        latency_ms=(time.monotonic() - started) * 1000,
    )
    value = row["prediction"]
    if row["status"] == "ok":
        valid = (
            (
                isinstance(value, dict)
                and set(value) == {"continue"}
                and type(value["continue"]) is bool
            )
            if item["task"] == "continuation"
            else validate(value, item)
        )
        if not valid:
            row["status"] = "invalid"
    write(path, row)
    return row


# cost: io up to 1494 CLI and 2394 HTTPS calls; basis: estimate
def main() -> None:
    setup()
    items = read(PRIVATE / "items.json")
    j2 = set(read(PRIVATE / "j2-ids.json"))
    tasks = [
        (i, c, r)
        for i in items
        for c in (
            ("B1", "L1")
            if i["task"] == "continuation"
            else ("J1", "L1", "J2")
            if i["id"] in j2
            else ("J1", "L1")
        )
        for r in (1, 2, 3)
    ]
    random.Random(SEED).shuffle(tasks)
    hashes = {
        i["id"]: hashlib.sha256(
            json.dumps(
                continuation_body(i)["state"]
                if i["task"] == "continuation"
                else state(i),
                ensure_ascii=False,
                sort_keys=True,
            ).encode()
        ).hexdigest()
        for i in items
    }
    seal = PRIVATE / "input-hashes.json"
    if seal.exists() and read(seal) != hashes:
        raise RuntimeError("input seal mismatch")
    write(seal, hashes)
    pending = [
        t
        for t in tasks
        if not (PRIVATE / "workflows" / f"{t[0]['id']}-{t[1]}-r{t[2]}.json").exists()
    ]
    # 첫 입력에서 공식 CLI 계약을 확인한 뒤 같은 봉인 조건을 계속한다.
    first = next((t for t in pending if t[1] == "L1"), None)
    if first:
        r = trial(first)
        print(
            json.dumps(
                {
                    "preflight": r["status"],
                    "models": list((r.get("model_usage") or {}).keys()),
                    "num_turns": r.get("num_turns"),
                }
            ),
            flush=True,
        )
        if not r.get("model_usage") or r.get("num_turns") != 1:
            raise RuntimeError("first claude process contract failed; inspect receipt")
        pending.remove(first)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = [pool.submit(trial, t) for t in pending]
        for n, future in enumerate(concurrent.futures.as_completed(futures), 1):
            try:
                row = future.result()
            except RuntimeError:
                STOP.set()
                for pending_future in futures:
                    pending_future.cancel()
                raise
            if n % 25 == 0:
                print(
                    json.dumps(
                        {
                            "completed": n,
                            "pending_at_start": len(pending),
                            "last_status": row["status"],
                        }
                    ),
                    flush=True,
                )


if __name__ == "__main__":
    main()
