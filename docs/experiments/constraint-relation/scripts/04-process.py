"""원응답을 후보쌍 관측과 후보 밖 정답으로 정규화한다."""

from __future__ import annotations

import json

from candidates import select_candidates
from support import (
    PRIVATE,
    load_original,
    read_json,
    read_rows,
    write_json,
    source_manifest,
)
from trials import build_trials, load_extension
from seal import seal_trials


def pair_rows(trial: dict, labels: dict, receipts: dict) -> tuple[list[dict], dict]:
    meta = trial["meta"]
    cid, tid = meta["conversation_id"], meta["turn_id"]
    gold = labels[cid].get(tid)
    candidates = meta["candidate_ids"]
    info = dict(meta, status=trial.get("status", "ready"))
    if not gold or gold["ambiguous"]:
        return [], dict(info, ambiguous_current=True)
    info["gold_targets"] = len(gold["targets"])
    info["covered"] = sum(t in candidates for t in gold["targets"])
    info["operation"] = gold["operation"]
    before_ids = meta.get("before_candidate_ids", candidates)
    info["coverage_by_operation"] = {}
    for operation in ("all", "replace", "release", "partial"):
        targets = gold["targets"] if operation in ("all", gold["operation"]) else []
        info["coverage_by_operation"][operation] = {
            "n": len(targets),
            "before": sum(t in before_ids for t in targets),
            "after": sum(t in candidates for t in targets),
        }
    rows = []
    found = set()
    for index, target in enumerate(candidates):
        earlier = labels[cid].get(target)
        if not earlier or earlier["ambiguous"]:
            continue
        responses = [receipts.get(trial["trial_id"] + f"-r{r}", {}) for r in (1, 2, 3)]
        probabilities = [
            [
                r.get("probabilities", {}).get(f"replaces_{index}"),
                r.get("probabilities", {}).get(f"releases_{index}"),
            ]
            for r in responses
        ]
        operation = gold["operation"] if target in gold["targets"] else "compatible"
        rows.append(
            dict(
                cohort=meta["cohort"],
                condition=meta["condition"],
                conversation_id=cid,
                turn_id=tid,
                target=target,
                distance=int(tid[2:]) - int(target[2:]),
                gold=operation,
                probabilities=probabilities,
                measured=True,
                candidate_included=True,
                trial_id=trial["trial_id"],
                ts_utc=responses[0].get("ts_utc"),
            )
        )
        found.add(target)
    for target in gold["targets"]:
        if target not in found:
            rows.append(
                dict(
                    cohort=meta["cohort"],
                    condition=meta["condition"],
                    conversation_id=cid,
                    turn_id=tid,
                    target=target,
                    distance=int(tid[2:]) - int(target[2:]),
                    gold=gold["operation"],
                    probabilities=[[None, None]] * 3,
                    measured=False,
                    candidate_included=target in candidates,
                    trial_id=trial["trial_id"],
                    ts_utc=None,
                )
            )
    return rows, info


def full_coverage(conversations: list[dict], labels: dict, receipts: dict) -> dict:
    counts = {
        c: {
            op: {"n": 0, "before": 0, "after": 0}
            for op in ("all", "replace", "release", "partial")
        }
        for c in ("C1", "C2", "C3")
    }
    full = {
        (t["meta"]["condition"], t["meta"]["conversation_id"], t["meta"]["turn_id"]): t
        for t in build_trials("original", full=True)
    }
    for c in conversations:
        cid, pool = c["conversation_id"], []
        for turn in c["turns"]:
            tid = turn["turn_id"]
            gold = labels[cid].get(tid)
            if gold and not gold["ambiguous"]:
                for condition in counts:
                    before = {
                        t["turn_id"] for t in select_candidates(turn, pool, condition)
                    }
                    after = full[condition, cid, tid]["meta"]["candidate_ids"]
                    for target in gold["targets"]:
                        for op in ("all", gold["operation"]):
                            counts[condition][op]["n"] += 1
                            counts[condition][op]["before"] += target in before
                            counts[condition][op]["after"] += target in after
            p = (
                receipts.get(f"base-{cid}-{tid}-r1", {})
                .get("probabilities", {})
                .get("is_constraint")
            )
            if p is not None and p >= 0.5:
                pool.append(turn)
    return counts


def main() -> None:
    if source_manifest() != read_json(PRIVATE / "plan.json")["source_manifest"]:
        raise RuntimeError("source changed")
    original, labels, old_receipts = load_original()
    _, extension_labels = load_extension()
    receipts = {r["trial_id"]: r for r in read_rows(PRIVATE / "jev.jsonl")}
    rows, infos = [], []
    for cohort, gold in [("original", labels), ("extension", extension_labels)]:
        trials = build_trials(cohort)
        seal_trials(cohort, trials)
        for trial in trials:
            pairs, info = pair_rows(trial, gold, receipts)
            rows.extend(pairs)
            infos.append(info)
    sample = {tuple(x) for x in read_json(PRIVATE / "plan.json")["sample"]}
    for cid, tid in sorted(sample):
        first = old_receipts.get(f"relation-{cid}-{tid}-r1", {})
        meta = dict(
            first.get("meta", {}),
            cohort="original",
            condition="C0",
            conversation_id=cid,
            turn_id=tid,
            candidate_ids=first.get("meta", {}).get("candidate_ids", []),
        )
        trial = {"meta": meta, "trial_id": f"relation-{cid}-{tid}"}
        pairs, info = pair_rows(trial, labels, old_receipts)
        rows.extend(pairs)
        infos.append(info)
    run_id = read_json(PRIVATE / "run.json")["run_id"]
    path = PRIVATE / "processed.jsonl"
    with path.open("w") as stream:
        for row in rows:
            stream.write(
                json.dumps(dict(row, run_id=run_id), ensure_ascii=False) + "\n"
            )
    write_json(PRIVATE / "trial-info.json", infos)
    write_json(PRIVATE / "coverage.json", full_coverage(original, labels, old_receipts))
    print({"processed_pairs_and_misses": len(rows)})


if __name__ == "__main__":
    main()
