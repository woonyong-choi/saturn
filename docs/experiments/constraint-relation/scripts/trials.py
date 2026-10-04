"""고정 표본의 후보와 정답을 별도 경로에서 읽는다."""

from __future__ import annotations

import random

from candidates import make_trial
from support import PRIVATE, SEED, SOURCE, load_original, read_json


def load_extension() -> tuple[list[dict], dict]:
    plan = read_json(PRIVATE / "plan.json")
    conversations, labels = [], {}
    for index, ids in enumerate(plan["label_batches"]):
        path = PRIVATE / "extension" / f"adjudicated-{index}.json"
        if not path.exists():
            continue
        batch = read_json(path)
        if batch.get("invalid_batch"):
            continue
        for cid in ids:
            conversations.append(read_json(SOURCE / "conversations" / (cid + ".json")))
            labels[cid] = {r["turn_id"]: r for r in batch[cid]["turns"]}
    return conversations, labels


def build_trials(cohort: str, *, full: bool = False) -> list[dict]:
    if cohort == "original":
        conversations, _, receipts = load_original()
        selected = {tuple(pair) for pair in read_json(PRIVATE / "plan.json")["sample"]}
        labels = {}
    else:
        conversations, labels = load_extension()
        receipts = {}
        projects = read_json(SOURCE / "selection.json")["projects"]
        population = [
            (m["conversation_id"], f"u-{i + 1:04d}")
            for m in sorted(projects, key=lambda m: m["conversation_id"])
            if 0 < m["user_turns"] < 10
            for i in range(m["user_turns"])
        ]
        selected = set(random.Random(SEED + 1).sample(population, 44))
    trials = []
    for c in sorted(conversations, key=lambda c: c["conversation_id"]):
        cid, pool = c["conversation_id"], []
        for turn in c["turns"]:
            tid = turn["turn_id"]
            if full or (cid, tid) in selected:
                for condition in ("C1", "C2", "C3"):
                    meta = {
                        "cohort": cohort,
                        "condition": condition,
                        "conversation_id": cid,
                        "turn_id": tid,
                    }
                    trials.append(make_trial(turn, pool, meta))
            if cohort == "original":
                probability = (
                    receipts.get(f"base-{cid}-{tid}-r1", {})
                    .get("probabilities", {})
                    .get("is_constraint")
                )
                include = probability is not None and probability >= 0.5
            else:
                label = labels[cid][tid]
                include = label["is_constraint"] and not label["ambiguous"]
            if include:
                pool.append(turn)
    return trials
