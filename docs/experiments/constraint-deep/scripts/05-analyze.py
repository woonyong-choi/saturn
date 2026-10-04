"""비공개 고정 원자료만으로 공개 집계와 보고서를 다시 만든다."""

from __future__ import annotations

import csv
import hashlib
import json
import random
import statistics
from collections import Counter

from analysis_metrics import (
    build_rows,
    compare_questions,
    final_metrics,
    label_agreement,
    load_labels,
    relation_metrics,
    relationship_repeats,
    repeat_metrics,
)
from judge import load_conversations, load_receipts
from labels import signature
from report import write_report
from statistics_metrics import (
    binomial_tail,
    cluster_bootstrap,
    metrics,
    power_requirement,
    ratio,
)
from storage import (
    MODELS,
    PRIVATE,
    PUBLIC,
    SEED,
    read_json,
    read_rows,
    write_json,
)


def curve_metrics(rows: list[dict], all_rows: list[dict]) -> list[dict]:
    curve = []
    for index in range(10):
        threshold = round(0.5 + 0.05 * index, 2)
        result = metrics(rows, threshold)
        result["all_input_ask_rate"] = ratio(
            sum(
                row["probability"] is not None and 0.5 <= row["probability"] < threshold
                for row in all_rows
            ),
            len(all_rows),
        )
        result["cluster_bootstrap"] = cluster_bootstrap(rows, threshold)
        curve.append(result)
    return curve


def confidence_metrics(rows: list[dict]) -> list[dict]:
    bands = []
    for index in range(10):
        members = [
            row
            for row in rows
            if row["probability"] is not None
            and index / 10 <= row["probability"]
            and (row["probability"] < (index + 1) / 10 or index == 9)
        ]
        result = ratio(sum(row["gold"] for row in members), len(members))
        bands.append(
            {
                "low": index / 10,
                "high": (index + 1) / 10,
                "precision": result,
                "meets_half_width": result["half_width"] is not None
                and result["half_width"] <= 0.1,
                "worst_case_n_shortfall": max(0, 93 - len(members)),
            }
        )
    return bands


def response_diagnostics(receipts: dict) -> list[dict]:
    groups = {}
    for row in receipts.values():
        key = row["meta"]["kind"], row["status"], row.get("http_status")
        size = len(json.dumps(row["request"], ensure_ascii=False).encode())
        groups.setdefault(key, []).append(size)
    return [
        {
            "kind": key[0],
            "status": key[1],
            "http_status": key[2],
            "calls": len(sizes),
            "request_bytes_min": min(sizes),
            "request_bytes_median": statistics.median(sizes),
            "request_bytes_max": max(sizes),
        }
        for key, sizes in sorted(groups.items(), key=lambda item: str(item[0]))
    ]


def hypotheses(curve: list[dict]) -> dict:
    at_eighty = next(row for row in curve if row["threshold"] == 0.8)
    entries = []
    for name, key in [("H1", "precision"), ("H2", "recall")]:
        value = at_eighty[key]
        p = binomial_tail(value["k"], value["n"], 0.8)
        entries.append({"name": name, "metric": key, **value, "p_value": p})
    previous = 0
    for index, entry in enumerate(sorted(entries, key=lambda row: row["p_value"])):
        entry["holm_p_value"] = min(1, max(previous, entry["p_value"] * (2 - index)))
        previous = entry["holm_p_value"]
        low, high = entry["ci95"]
        entry["decision"] = (
            "채택"
            if low is not None and low > 0.8 and entry["holm_p_value"] <= 0.05
            else "기각"
            if high is not None and high < 0.8
            else "보류"
        )
    return {entry["name"]: entry for entry in entries}


def review_cases(conversations: list[dict], labels: dict, rows: list[dict]) -> int:
    by_key = {(row["conversation_id"], row["turn_id"]): row for row in rows}
    disagreements, boundaries = [], []
    for conversation in conversations:
        cid = conversation["conversation_id"]
        for turn in conversation["turns"]:
            lanes = {
                model: labels[cid][model].get(turn["turn_id"])
                for model in (*MODELS, "adjudicated")
            }
            values = [lanes[model] for model in MODELS]
            row = by_key[cid, turn["turn_id"]]
            case = {
                "conversation_id": cid,
                "length_band": conversation["length_band"],
                "turn": turn,
                "labels": lanes,
                "probability": row["probability"],
                "human_review_status": "pending",
            }
            if all(values) and signature(values[0]) != signature(values[1]):
                disagreements.append(case)
            elif row["ambiguous"] or (
                row["probability"] is not None and 0.5 <= row["probability"] <= 0.85
            ):
                boundaries.append(case)
    randomizer = random.Random(SEED)
    randomizer.shuffle(disagreements)
    randomizer.shuffle(boundaries)
    chosen = (disagreements + boundaries)[:40]
    target = PRIVATE / "human-review.jsonl"
    target.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in chosen)
    )
    return len(chosen)


# cost: time O(n), heap O(1), io 2 files; vars: n = aggregate rows; basis: estimate
def write_tables(summary: dict) -> None:
    target = PUBLIC / "results/tables"
    target.mkdir(parents=True, exist_ok=True)
    with (target / "thresholds.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(
            [
                "threshold",
                "n",
                "tp",
                "fp",
                "fn",
                "tn",
                "precision",
                "precision_low",
                "precision_high",
                "recall",
                "f1",
                "ask_rate",
            ]
        )
        for row in summary["threshold_curve"]:
            writer.writerow(
                [
                    row["threshold"],
                    row["n"],
                    row["tp"],
                    row["fp"],
                    row["fn"],
                    row["tn"],
                    row["precision"]["value"],
                    *row["precision"]["ci95"],
                    row["recall"]["value"],
                    row["f1"],
                    row["all_input_ask_rate"]["value"],
                ]
            )
    with (target / "confidence.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(
            [
                "low",
                "high",
                "positives",
                "n",
                "precision",
                "ci_low",
                "ci_high",
                "half_width",
                "target_met",
            ]
        )
        for row in summary["confidence_bands"]:
            value = row["precision"]
            writer.writerow(
                [
                    row["low"],
                    row["high"],
                    value["k"],
                    value["n"],
                    value["value"],
                    *value["ci95"],
                    value["half_width"],
                    row["meets_half_width"],
                ]
            )


def write_manifest() -> str:
    names = [
        "census.json",
        "selection.json",
        "calls.jsonl",
        "jev.jsonl",
        "redaction-repair.json",
        "human-review.jsonl",
    ]
    paths = [PRIVATE / name for name in names if (PRIVATE / name).exists()]
    paths += sorted((PRIVATE / "conversations").glob("*.json"))
    paths += sorted((PRIVATE / "labels").glob("*/*.jsonl"))
    paths += sorted((PRIVATE / "codex").glob("*/*"))
    manifest = "".join(
        hashlib.sha256(path.read_bytes()).hexdigest()
        + "  "
        + str(path.relative_to(PRIVATE))
        + "\n"
        for path in paths
        if path.is_file()
    )
    target = PUBLIC / "data/SHA256SUMS"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(manifest)
    return hashlib.sha256(target.read_bytes()).hexdigest()


# cost: time O(b + tn), heap O(b), io f files; vars: b = raw bytes, t = bootstrap draws, n = turns, f = artifacts; basis: estimate
def main() -> None:
    conversations = load_conversations()
    labels = load_labels(conversations)
    for conversation in conversations:
        for lane, rows in labels[conversation["conversation_id"]].items():
            if len(rows) != len(conversation["turns"]):
                raise RuntimeError(f"incomplete label lane: {lane}")
    receipts = load_receipts()
    all_rows = build_rows(conversations, labels, receipts)
    rows = [row for row in all_rows if row["gold"] is not None and not row["ambiguous"]]
    confirmatory_rows = [row for row in rows if not row["protocol_deviation"]]
    curve = curve_metrics(rows, all_rows)
    candidates = [
        row["threshold"]
        for row in curve
        if row["precision"]["n"]
        and row["precision"]["ci95"][0] >= 0.8
        and row["precision"]["half_width"] <= 0.1
    ]
    calls = read_rows(PRIVATE / "calls.jsonl")
    selection = read_json(PRIVATE / "selection.json")
    census = read_json(PRIVATE / "census.json")
    summary = {
        "design_commit": "8b7c236",
        "run_started": min(row["ts_utc"] for row in calls),
        "census": {
            key: value
            for key, value in selection["census"].items()
            if key != "saturn_turns"
        },
        "selection": {
            key: selection[key] for key in ("conversations_by_band", "turns_by_band")
        },
        "selected_projects": len(conversations),
        "selected_turns": len(all_rows),
        "saturn_turns": sum(
            len(row["turns"]) for row in conversations if row["is_saturn"]
        ),
        "exclusions": {
            "changed_files": len(selection["changed_files"]),
            "changed_file_turns": sum(
                row["user_turns"]
                for row in census["files"]
                if row["file_id"] in selection["changed_files"]
            ),
            "duplicate_turns": sum(
                row["duplicate_turns"] for row in selection["projects"]
            ),
            "short_project_turns": sum(
                row["user_turns"]
                for row in selection["projects"]
                if row["length_band"] is None
            ),
            "ambiguous_or_missing_gold": len(all_rows) - len(rows),
            "all_protocol_deviation_turns": sum(
                row["protocol_deviation"] for row in all_rows
            ),
            "confirmatory_protocol_deviation_turns": len(rows) - len(confirmatory_rows),
        },
        "calls": dict(Counter(row["kind"] for row in calls)),
        "redaction_repair": read_json(PRIVATE / "redaction-repair.json")
        if (PRIVATE / "redaction-repair.json").exists()
        else None,
        "interrupted_label_calls": sum(
            row["kind"] == "codex"
            and not (PRIVATE / "codex" / row["trial_id"] / "receipt.json").exists()
            for row in calls
        ),
        "jev_statuses": dict(Counter(row["status"] for row in receipts.values())),
        "response_diagnostics": response_diagnostics(receipts),
        "jev_by_kind": dict(Counter(row["meta"]["kind"] for row in receipts.values())),
        "jev_models": dict(
            Counter(row.get("model") or "missing" for row in receipts.values())
        ),
        "label_agreement": label_agreement(conversations, labels),
        "threshold_curve": curve,
        "confidence_bands": confidence_metrics(rows),
        "hypotheses": hypotheses([metrics(confirmatory_rows, 0.8)]),
        "confirmatory_turns": len(confirmatory_rows),
        "recommended_threshold": min(candidates) if candidates else None,
        "power": power_requirement(),
        "precision_target_minimum": 93,
        "strata": {
            key: {
                value: metrics([row for row in rows if row[key] == value], 0.8)
                for value in sorted({row[key] for row in rows})
            }
            for key in ("length_band", "conversation_id", "language")
        },
        "repeats": repeat_metrics(all_rows, receipts, 0.8),
        "relationship_repeats": relationship_repeats(receipts),
        "alternative": compare_questions(rows, receipts),
        "relations": relation_metrics(conversations, labels, receipts),
        "final_sets": [
            final_metrics(conversations, labels, receipts, row["threshold"])
            for row in curve
        ],
        "human_review_cases": review_cases(conversations, labels, all_rows),
        "conditional_valid_response_metrics": metrics(
            [row for row in rows if row["probability"] is not None], 0.8
        ),
    }
    summary["label_calls_by_lane"] = {
        lane: sum(
            row["kind"] == "codex" and row["trial_id"].startswith(lane + "-")
            for row in calls
        )
        for lane in (*MODELS, "adjudicated")
    }
    summary["manifest_sha256"] = write_manifest()
    write_json(PUBLIC / "results/summary.json", summary)
    write_tables(summary)
    write_report(summary)
    print(
        json.dumps(
            {
                "analyzed_turns": len(rows),
                "calls": summary["calls"],
                "recommended_threshold": summary["recommended_threshold"],
            }
        )
    )


if __name__ == "__main__":
    main()
