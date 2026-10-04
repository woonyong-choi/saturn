"""기록 유무와 질문 형식을 교차하여 요청을 세 번씩 수집한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import math
import os
import random

from common import (
    PRIVATE,
    SEED,
    append_row,
    initialize,
    judge,
    read_json,
    read_rows,
    state,
    write_json,
)


def parse_answers(reply: dict, questions: dict) -> dict:
    answers = reply.get("answers", {}) if isinstance(reply, dict) else {}
    if not isinstance(answers, dict):
        answers = {}
    parsed = {}
    for name, q in questions.items():
        a = answers.get(name, {})
        if not isinstance(a, dict):
            a = {}
        p = a.get("probabilities") if q["type"] == "choice" else a.get("noul")
        vals = list(p.values()) if isinstance(p, dict) else [p]
        ok = bool(vals) and all(
            type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1 for v in vals
        )
        if q["type"] == "choice":
            ok = (
                ok
                and isinstance(p, dict)
                and set(p) == set(q["criteria"])
                and abs(sum(vals) - 1) < 0.01
            )
        if q["type"] != "choice":
            ok = ok and type(p) in (int, float)
        parsed[name] = p if ok else None
    return parsed


def trial(item: dict, form: str, record: bool) -> dict:
    s = state(item, record)
    if form == "pair":
        questions = {
            "is_release": judge.question(
                "The user's latest input asks to stop following an existing constraint, wholly, partly, or temporarily, rather than cancel or stop a task."
            ),
            "is_constraint": judge.question(judge.BASE_QUESTION),
        }
        for i, r in enumerate(item["rules"]):
            questions[f"releases_{i + 1}"] = judge.question(
                f"For earlier constraint c{i + 1} and the latest user input: "
                + judge.RELEASE_QUESTION
            )
    else:
        questions = {
            "release_target": {
                "type": "choice",
                "instructions": "Does the latest user input ask to stop following one of the active constraints, wholly, partly, or temporarily? Select its constraint ID, or none if it does not request release of a constraint. Canceling or stopping a task is not releasing a constraint.",
                "criteria": {
                    **{f"c{i + 1}": None for i in range(item["n"])},
                    "none": None,
                },
            }
        }
    return dict(
        trial_id=f"{item['id']}-{form}-{int(record)}",
        model=judge.MODEL,
        state=s,
        questions=questions,
        meta=dict(
            item_id=item["id"],
            form=form,
            record=record,
            n=item["n"],
            category=item["category"],
            source=item["source"],
        ),
    )


# cost: io 12n HTTPS requests, capped at 5000; vars: n = retained inputs; basis: estimate
def main() -> None:
    initialize()
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("judge key missing")
    if (PRIVATE / "stopped.json").exists():
        raise RuntimeError("collection stopped")
    judge.parse_answers = parse_answers
    items = read_json(PRIVATE / "items.json")
    trials = [
        trial(i, f, r) for i in items for f in ("pair", "choice") for r in (False, True)
    ]
    hashes = {
        t["trial_id"]: hashlib.sha256(
            json.dumps(
                {k: t[k] for k in ("model", "state", "questions")}, ensure_ascii=False
            ).encode()
        ).hexdigest()
        for t in trials
    }
    seal = PRIVATE / "request-hashes.json"
    if seal.exists() and read_json(seal) != hashes:
        raise RuntimeError("sealed request mismatch")
    write_json(seal, hashes)
    rng = random.Random(SEED)
    rng.shuffle(trials)
    reserved = {r["trial_id"] for r in read_rows(PRIVATE / "calls.jsonl")}
    run = read_json(PRIVATE / "run.json")
    pending = []
    for t in trials:
        body = json.dumps(
            {k: t[k] for k in ("model", "state", "questions")}, ensure_ascii=False
        ).encode()
        if len(body) > 100000:
            raise RuntimeError("request exceeds byte limit")
        pending.extend(
            (t, r, len(body))
            for r in (1, 2, 3)
            if f"{t['trial_id']}-r{r}" not in reserved
        )
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        for start in range(0, len(pending), 8):
            batch = pending[start : start + 8]
            for task, row in zip(
                batch,
                executor.map(lambda task: judge.post_trial(task[0], task[1]), batch),
            ):
                t, _, size = task
                row.update(
                    run_id=run["run_id"],
                    condition=f"{t['meta']['form']}-{int(t['meta']['record'])}",
                    request_bytes=size,
                )
                append_row(PRIVATE / "jev.jsonl", row)
                if row.get("http_status") in (401, 403) or row.get("model_unavailable"):
                    write_json(
                        PRIVATE / "stopped.json",
                        dict(
                            reason="authentication or model rejected",
                            trial_id=row["trial_id"],
                        ),
                    )
            if (PRIVATE / "stopped.json").exists():
                raise RuntimeError("authentication or model rejected")
            if start % 200 == 0:
                print(
                    json.dumps(
                        dict(phase="jev", done=start + len(batch), total=len(pending))
                    ),
                    flush=True,
                )


if __name__ == "__main__":
    main()
