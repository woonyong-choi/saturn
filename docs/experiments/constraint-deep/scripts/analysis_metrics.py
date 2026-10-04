"""라벨 합의·반복·관계·끝 집합의 진단 지표를 계산한다."""

from __future__ import annotations

import random
import re
import statistics

from labels import replay_labels, signature
from statistics_metrics import binomial_tail, metrics, percentile, ratio
from storage import MODELS, PRIVATE, SEED, read_rows


def load_labels(conversations: list[dict]) -> dict:
    result = {}
    for conversation in conversations:
        cid = conversation["conversation_id"]
        result[cid] = {}
        for lane in (*MODELS, "adjudicated"):
            chunks = read_rows(PRIVATE / "labels" / lane / (cid + ".jsonl"))
            result[cid][lane] = flatten_label_chunks(chunks)
    return result


def flatten_label_chunks(chunks: list[dict]) -> dict:
    rows = {}
    affected = False
    for chunk in chunks:
        affected |= chunk.get("protocol_deviation", False)
        for row in chunk["turns"]:
            rows[row["turn_id"]] = {**row, "protocol_deviation": affected}
    return rows


def classify_relation(replace: float | None, release: float | None) -> str:
    if replace is None or release is None:
        return "invalid"
    if release is not None and release >= 0.8:
        return "release"
    if replace is not None and replace >= 0.8:
        return "replace"
    return "partial" if replace >= 0.5 else "compatible"


def language(text: str) -> str:
    if re.search("[가-힣]", text):
        return "ko"
    return "en" if re.search("[A-Za-z]", text) else "other"


def build_rows(conversations: list[dict], labels: dict, receipts: dict) -> list[dict]:
    rows = []
    for conversation in conversations:
        cid = conversation["conversation_id"]
        for turn in conversation["turns"]:
            label = labels[cid]["adjudicated"].get(turn["turn_id"])
            trial_id = f"base-{cid}-{turn['turn_id']}"
            response = receipts.get(trial_id + "-r1", {})
            rows.append(
                {
                    "conversation_id": cid,
                    "turn_id": turn["turn_id"],
                    "trial_id": trial_id,
                    "length_band": conversation["length_band"],
                    "language": language(turn["text"]),
                    "gold": label["is_constraint"] if label else None,
                    "ambiguous": label["ambiguous"] if label else True,
                    "probability": response.get("probabilities", {}).get(
                        "is_constraint"
                    ),
                    "status": response.get("status", "missing"),
                    "protocol_deviation": any(
                        labels[cid][lane]
                        .get(turn["turn_id"], {})
                        .get("protocol_deviation", False)
                        for lane in (*MODELS, "adjudicated")
                    ),
                }
            )
    return rows


def label_agreement(conversations: list[dict], labels: dict) -> dict:
    full, classification, final, by_band = [], [], [], {}
    for conversation in conversations:
        cid = conversation["conversation_id"]
        lanes = labels[cid]
        left, right = (lanes[model] for model in MODELS)
        shared = set(left) & set(right)
        exact = [
            signature(left[tid]) == signature(right[tid]) for tid in sorted(shared)
        ]
        full.extend(exact)
        classification.extend(
            left[tid]["is_constraint"] == right[tid]["is_constraint"] for tid in shared
        )
        by_band.setdefault(conversation["length_band"], []).extend(exact)
        if len(shared) == len(conversation["turns"]):
            endings = [
                read_rows(PRIVATE / "labels" / model / (cid + ".jsonl"))[-1]
                for model in MODELS
            ]
            if all(row.get("final_set_valid", True) for row in endings):
                final.append(
                    set(endings[0]["final_active_turn_ids"])
                    == set(endings[1]["final_active_turn_ids"])
                )
    return {
        "turn_full": ratio(sum(full), len(full)),
        "turn_classification": ratio(sum(classification), len(classification)),
        "final_set": ratio(sum(final), len(final)),
        "final_set_excluded_projects": len(conversations) - len(final),
        "by_length": {
            band: ratio(sum(values), len(values)) for band, values in by_band.items()
        },
        "final_set_invalid_chunks": sum(
            not chunk.get("final_set_valid", True)
            for lane in (*MODELS, "adjudicated")
            for conversation in conversations
            for chunk in read_rows(
                PRIVATE / "labels" / lane / (conversation["conversation_id"] + ".jsonl")
            )
        ),
        "partial_output_chunks": sum(
            chunk.get("partial_output", False)
            for lane in (*MODELS, "adjudicated")
            for conversation in conversations
            for chunk in read_rows(
                PRIVATE / "labels" / lane / (conversation["conversation_id"] + ".jsonl")
            )
        ),
        "partial_output_retained_turns": sum(
            len(chunk["turns"])
            for lane in (*MODELS, "adjudicated")
            for conversation in conversations
            for chunk in read_rows(
                PRIVATE / "labels" / lane / (conversation["conversation_id"] + ".jsonl")
            )
            if chunk.get("partial_output")
        ),
        "structured_output_chunks": sum(
            chunk.get("structured_output", False)
            for lane in (*MODELS, "adjudicated")
            for conversation in conversations
            for chunk in read_rows(
                PRIVATE / "labels" / lane / (conversation["conversation_id"] + ".jsonl")
            )
        ),
        "adjudicated_turns": sum(
            len(lanes["adjudicated"]) for lanes in labels.values()
        ),
        "unresolved_turns": sum(
            row["ambiguous"]
            for lanes in labels.values()
            for row in lanes["adjudicated"].values()
        ),
    }


def repeat_metrics(rows: list[dict], receipts: dict, threshold: float) -> dict:
    agreement, deviations = [], []
    for row in rows:
        probabilities = [
            receipts.get(row["trial_id"] + f"-r{repeat}", {})
            .get("probabilities", {})
            .get("is_constraint")
            for repeat in (1, 2, 3)
        ]
        if any(value is None for value in probabilities):
            continue
        agreement.append(len({value >= threshold for value in probabilities}) == 1)
        deviations.append(statistics.pstdev(probabilities))
    return {
        "classification": ratio(sum(agreement), len(agreement)),
        "mean_probability_sd": statistics.mean(deviations) if deviations else None,
        "incomplete_triplets": len(rows) - len(agreement),
    }


def relationship_repeats(receipts: dict) -> dict:
    agreements = []
    incomplete = 0
    for first in receipts.values():
        if first["meta"]["kind"] != "relation" or first["repeat"] != 1:
            continue
        prefix = first["trial_id"][:-1]
        rows = [receipts.get(prefix + str(repeat), {}) for repeat in (1, 2, 3)]
        for index, _ in enumerate(first["meta"]["candidate_ids"]):
            outcomes = []
            for row in rows:
                replace = row.get("probabilities", {}).get(f"replaces_{index}")
                release = row.get("probabilities", {}).get(f"releases_{index}")
                if replace is None or release is None:
                    continue
                outcomes.append(classify_relation(replace, release))
            if len(outcomes) == 3:
                agreements.append(len(set(outcomes)) == 1)
            else:
                incomplete += 1
    return {
        "classification": ratio(sum(agreements), len(agreements)),
        "incomplete_triplets": incomplete,
    }


def compare_questions(rows: list[dict], receipts: dict) -> dict:
    paired = []
    for row in rows:
        trial = row["trial_id"].replace("base-", "alternative-", 1) + "-r1"
        alt = receipts.get(trial, {}).get("probabilities", {}).get("is_constraint")
        if alt is None or row["probability"] is None:
            continue
        baseline_correct = (row["probability"] >= 0.8) == row["gold"]
        alternative_correct = (alt >= 0.8) == row["gold"]
        paired.append(
            (baseline_correct, alternative_correct, {**row, "probability": alt})
        )
    b = sum(a and not c for a, c, _ in paired)
    c = sum(not a and d for a, d, _ in paired)
    discordant = b + c
    p = min(1.0, 2 * binomial_tail(max(b, c), discordant, 0.5)) if discordant else 1.0
    method = "exact McNemar (two-sided binomial)"
    differences = [int(alt) - int(base) for base, alt, _ in paired]
    randomizer = random.Random(SEED)
    samples = (
        [
            statistics.mean(randomizer.choices(differences, k=len(differences)))
            for _ in range(2000)
        ]
        if differences
        else []
    )
    return {
        "n": len(paired),
        "both_correct": sum(a and b for a, b, _ in paired),
        "baseline_only": b,
        "alternative_only": c,
        "both_wrong": sum(not a and not b for a, b, _ in paired),
        "p_value": p,
        "method": method,
        "accuracy_difference": statistics.mean(differences) if differences else None,
        "difference_ci95": [percentile(samples, 0.025), percentile(samples, 0.975)],
        "alternative_metrics": metrics([row for _, _, row in paired], 0.8),
        "baseline_accuracy": ratio(sum(a for a, _, _ in paired), len(paired)),
        "alternative_accuracy": ratio(sum(b for _, b, _ in paired), len(paired)),
    }


def distance_band(distance: int) -> str:
    for upper, name in (
        (5, "1-5"),
        (20, "6-20"),
        (100, "21-100"),
        (float("inf"), "101+"),
    ):
        if distance <= upper:
            return name
    raise ValueError("invalid distance")


def relation_metrics(conversations: list[dict], labels: dict, receipts: dict) -> dict:
    pairs, uncovered = [], []
    for conversation in conversations:
        cid = conversation["conversation_id"]
        gold = labels[cid]["adjudicated"]
        for turn in conversation["turns"]:
            current = gold.get(turn["turn_id"])
            if not current or current["ambiguous"]:
                continue
            row = receipts.get(f"relation-{cid}-{turn['turn_id']}-r1", {})
            candidates = row.get("meta", {}).get("candidate_ids", [])
            for target in current["targets"]:
                if target not in candidates:
                    uncovered.append(
                        {
                            "operation": current["operation"],
                            "distance": int(turn["turn_id"][2:]) - int(target[2:]),
                        }
                    )
            for index, target in enumerate(candidates):
                earlier = gold.get(target)
                if not earlier or earlier["ambiguous"]:
                    continue
                probability = row.get("probabilities", {}).get(f"replaces_{index}")
                release = row.get("probabilities", {}).get(f"releases_{index}")
                operation = (
                    current["operation"]
                    if target in current["targets"]
                    else "compatible"
                )
                predicted = classify_relation(probability, release)
                pairs.append(
                    {
                        "gold": operation,
                        "predicted": predicted,
                        "probability": probability,
                        "release": release,
                        "distance": distance_band(
                            int(turn["turn_id"][2:]) - int(target[2:])
                        ),
                    }
                )
    by_operation = {}
    for operation in ("replace", "release"):
        binary = [
            {
                "gold": row["gold"] == operation,
                "probability": float(row["predicted"] == operation),
            }
            for row in pairs
        ]
        by_operation[operation] = metrics(binary, 0.8)
    return {
        "pairs": len(pairs),
        "classification": ratio(
            sum(row["gold"] == row["predicted"] for row in pairs), len(pairs)
        ),
        "by_operation": by_operation,
        "by_distance": {
            band: ratio(
                sum(
                    row["gold"] == row["predicted"]
                    for row in pairs
                    if row["distance"] == band
                ),
                sum(row["distance"] == band for row in pairs),
            )
            for band in ("1-5", "6-20", "21-100", "101+")
        },
        "uncovered_gold_targets": len(uncovered),
        "gold_transition_targets": sum(
            len(row["targets"])
            for lanes in labels.values()
            for row in lanes["adjudicated"].values()
            if not row["ambiguous"]
        ),
        "invalid_pairs": sum(
            row["probability"] is None or row["release"] is None for row in pairs
        ),
    }


def final_metrics(
    conversations: list[dict], labels: dict, receipts: dict, threshold: float
) -> dict:
    results = []
    for conversation in conversations:
        cid = conversation["conversation_id"]
        gold = labels[cid]["adjudicated"]
        if len(gold) != len(conversation["turns"]):
            continue
        predicted, incomplete = set(), False
        for turn in conversation["turns"]:
            tid = turn["turn_id"]
            probability = (
                receipts.get(f"base-{cid}-{tid}-r1", {})
                .get("probabilities", {})
                .get("is_constraint")
            )
            relation = receipts.get(f"relation-{cid}-{tid}-r1", {})
            candidates = relation.get("meta", {}).get("candidate_ids", [])
            incomplete |= probability is None or bool(predicted - set(candidates))
            for index, target in enumerate(candidates):
                release = relation.get("probabilities", {}).get(f"releases_{index}")
                replace = relation.get("probabilities", {}).get(f"replaces_{index}")
                operation = classify_relation(replace, release)
                incomplete |= operation == "invalid"
                if operation == "release":
                    predicted.discard(target)
                elif (
                    probability is not None
                    and probability >= threshold
                    and operation == "replace"
                ):
                    predicted.discard(target)
            if probability is not None and probability >= threshold:
                predicted.add(tid)
        expected = replay_labels(set(), list(gold.values()))
        union = predicted | expected
        results.append(
            {
                "is_exact": predicted == expected,
                "jaccard": len(predicted & expected) / len(union) if union else 1,
                "incomplete": incomplete,
                "ambiguous": any(row["ambiguous"] for row in gold.values()),
                "predicted_count": len(predicted),
                "gold_count": len(expected),
            }
        )
    complete = [
        row for row in results if not row["incomplete"] and not row["ambiguous"]
    ]
    return {
        "threshold": threshold,
        "provisional_exact": ratio(
            sum(row["is_exact"] for row in results), len(results)
        ),
        "complete_unambiguous_exact": ratio(
            sum(row["is_exact"] for row in complete), len(complete)
        ),
        "provisional_mean_jaccard": statistics.mean(row["jaccard"] for row in results)
        if results
        else None,
        "incomplete_projects": sum(row["incomplete"] for row in results),
        "ambiguous_projects": sum(row["ambiguous"] for row in results),
        "projects": results,
    }
