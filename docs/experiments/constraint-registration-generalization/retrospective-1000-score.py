"""두 번의 독립 판정과 재판정을 합쳐 1,000건 정답지 후보를 평가한다."""

from __future__ import annotations

import argparse
import collections
import importlib.util
import json
import random
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"script unavailable: {path.name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ret = load("retrospective", HERE / "retrospective.py")
audit = load("retrospective_analyze", HERE / "retrospective-analyze.py")
rounding = load("rounding_audit", HERE / "rounding_audit.py")
BASE = ret.OUT
QUALITY = BASE / "quality1000"


def load_with_retries(group: str) -> tuple[list[dict], dict, set[str], dict]:
    source = QUALITY / group
    cases, labels, original_audit = audit.load_labels(source)
    audits = {"original": original_audit}
    bad = set(original_audit["bad_evidence"])
    for sid in bad:
        labels.pop(sid, None)
    for number in (1, 2, 3):
        retry = source / f"retry{number}"
        if not retry.exists():
            continue
        _, recovered, retry_audit = audit.load_labels(retry)
        for sid in retry_audit["bad_evidence"]:
            recovered.pop(sid, None)
        if labels.keys() & recovered.keys():
            raise RuntimeError("duplicate retry judgment")
        labels.update(recovered)
        bad.difference_update(recovered.keys())
        bad.update(retry_audit["bad_evidence"])
        audits[f"retry{number}"] = retry_audit
    return cases, labels, bad, audits


def all_first_labels() -> tuple[dict, set[str], dict]:
    _, base, base_audit = audit.load_labels(BASE)
    for sid in base_audit["bad_evidence"]:
        base.pop(sid, None)
    _, primary, primary_bad, primary_audits = load_with_retries("primary")
    duplicate = base.keys() & primary.keys()
    if duplicate:
        raise RuntimeError("duplicate first judgment")
    return (
        base | primary,
        (set(base_audit["bad_evidence"]) - primary.keys()) | primary_bad,
        {
            "reused_raw": base_audit,
            "primary_raw": primary_audits,
        },
    )


def judgments() -> tuple[list[dict], dict, dict, set[str], set[str], dict]:
    cases = json.loads((QUALITY / "cases.json").read_text())
    first, first_bad, audits = all_first_labels()
    _, second, second_bad, second_audits = load_with_retries("secondary")
    audits["secondary_raw"] = second_audits
    ids = {case["id"] for case in cases}
    if ids - first.keys() or ids - second.keys():
        raise RuntimeError("first or second judgment incomplete")
    return cases, first, second, first_bad, second_bad, audits


def prepare_third() -> None:
    cases, first, second, first_bad, second_bad, audits = judgments()
    review = [
        case
        for case in cases
        if case["id"] in first_bad
        or case["id"] in second_bad
        or first[case["id"]]["label"] != second[case["id"]]["label"]
        or first[case["id"]]["label"] == "uncertain"
    ]
    ret.write_once(QUALITY / "third/cases.json", review)
    ret.write_once(
        QUALITY / "third/review-plan.json",
        {
            "ts_utc": ret.now(),
            "selected": len(cases),
            "direct_agreement": len(cases) - len(review),
            "third_requested": len(review),
            "first_bad_evidence": len(first_bad & {c["id"] for c in cases}),
            "second_bad_evidence": len(second_bad),
            "raw_audits": audits,
        },
    )
    print(
        json.dumps(
            {
                "third_requested": len(review),
                "direct_agreement": len(cases) - len(review),
            }
        )
    )


def probability_map() -> dict[str, float]:
    questions = json.loads((HERE / "questions.json").read_text())
    result = {}
    for path in (ret.PRIVATE / "raw").glob("*.json"):
        record = json.loads(path.read_text())
        values = rounding.recovered_probabilities(record, questions)
        if values is None:
            raise RuntimeError("unusable Jev response")
        result[record["sample_id"]] = values["korean"]
    return result


def cluster_low_interval(rows: list[dict]) -> list[float] | None:
    groups = collections.defaultdict(list)
    for row in rows:
        if row["stratum"] == "low" and row["gold"] != "uncertain":
            groups[row["project_id"]].append(row)
    if not groups:
        return None
    counts = [
        (sum(r["gold"] == "constraint" for r in group), len(group))
        for group in groups.values()
    ]
    rng = random.Random(38220261005)
    values = []
    for _ in range(10_000):
        sample = rng.choices(counts, k=len(counts))
        numerator = sum(item[0] for item in sample)
        denominator = sum(item[1] for item in sample)
        if denominator:
            values.append(numerator / denominator)
    return [audit.percentile(values, 0.025), audit.percentile(values, 0.975)]


def score() -> None:
    cases, first, second, first_bad, second_bad, raw_audits = judgments()
    third_cases, third, third_bad, third_audits = load_with_retries("third")
    if len(third_cases) != len(third):
        raise RuntimeError("third judgment incomplete")
    scores = probability_map()
    goldset = []
    for case in cases:
        sid = case["id"]
        votes = [
            ("first", first[sid], sid not in first_bad),
            ("second", second[sid], sid not in second_bad),
        ]
        if sid in third:
            votes.append(("third", third[sid], sid not in third_bad))
        valid = [vote for vote in votes if vote[2] and vote[1]["label"] != "uncertain"]
        chosen = next(
            (
                label
                for label in ("constraint", "not_constraint")
                if sum(vote[1]["label"] == label for vote in valid) >= 2
            ),
            "uncertain",
        )
        goldset.append(
            {
                "id": sid,
                "project_id": case["project_id"],
                "provider": case["provider"],
                "stratum": "automatic"
                if scores[sid] >= 0.77
                else "ask"
                if scores[sid] >= 0.43
                else "low",
                "jev_p": scores[sid],
                "gold": chosen,
                "votes": [
                    {
                        "pass": name,
                        "label": value["label"],
                        "category": value["category"],
                        "reason": value["reason"],
                        "evidence": value["evidence"],
                        "valid_evidence": okay,
                    }
                    for name, value, okay in votes
                ],
                "future_omitted_chars": case["future_coverage"]["omitted_chars"],
            }
        )
    by_stratum = {
        name: [row for row in goldset if row["stratum"] == name]
        for name in ("automatic", "ask", "low")
    }
    if [len(by_stratum[name]) for name in ("automatic", "ask", "low")] != [
        57,
        301,
        642,
    ]:
        raise RuntimeError("stratum count mismatch")
    strata = {
        name: {
            "sample": len(rows),
            "positive": sum(row["gold"] == "constraint" for row in rows),
            "negative": sum(row["gold"] == "not_constraint" for row in rows),
            "uncertain": sum(row["gold"] == "uncertain" for row in rows),
        }
        for name, rows in by_stratum.items()
    }
    auto, ask, low = (strata[name] for name in ("automatic", "ask", "low"))
    low_known = low["positive"] + low["negative"]
    if low_known == 0:
        raise RuntimeError("no known low score labels")
    low_rate = low["positive"] / low_known
    positive_estimate = auto["positive"] + ask["positive"] + 3642 * low_rate
    low_wilson = audit.wilson(low["positive"], low_known, 0.05)
    low_cluster = cluster_low_interval(goldset)
    known_above = auto["positive"] + ask["positive"]

    def recall_interval(numerator: int, low_interval: list[float]) -> list[float]:
        return [
            numerator / (known_above + 3642 * low_interval[1]),
            numerator / (known_above + 3642 * low_interval[0]),
        ]

    result = {
        "ts_utc": ret.now(),
        "selected": 1000,
        "population": 4000,
        "strata": strata,
        "first_second_labels": {
            f"{left}|{right}": count
            for (left, right), count in collections.Counter(
                (first[case["id"]]["label"], second[case["id"]]["label"])
                for case in cases
            ).items()
        },
        "gold_labels": dict(collections.Counter(row["gold"] for row in goldset)),
        "providers": dict(collections.Counter(row["provider"] for row in goldset)),
        "direct_agreement": 1000 - len(third_cases),
        "third_requested": len(third_cases),
        "third_valid": len(third),
        "consensus": sum(row["gold"] != "uncertain" for row in goldset),
        "future_truncated": sum(row["future_omitted_chars"] > 0 for row in goldset),
        "raw_audits": raw_audits | {"third_raw": third_audits},
        "metrics": {
            "automatic_precision_known": auto["positive"]
            / (auto["positive"] + auto["negative"])
            if auto["positive"] + auto["negative"]
            else None,
            "automatic_precision_bounds": [
                auto["positive"] / 57,
                (auto["positive"] + auto["uncertain"]) / 57,
            ],
            "automatic_precision_wilson95": audit.wilson(
                auto["positive"], auto["positive"] + auto["negative"], 0.05
            ),
            "automatic_recall_estimate": auto["positive"] / positive_estimate
            if positive_estimate
            else None,
            "automatic_recall_wilson95": recall_interval(auto["positive"], low_wilson),
            "automatic_recall_project_bootstrap95": recall_interval(
                auto["positive"], low_cluster
            ),
            "candidate_recall_estimate": (auto["positive"] + ask["positive"])
            / positive_estimate
            if positive_estimate
            else None,
            "candidate_recall_wilson95": recall_interval(known_above, low_wilson),
            "candidate_recall_project_bootstrap95": recall_interval(
                known_above, low_cluster
            ),
            "automatic_recall_label_bounds": [
                auto["positive"]
                / (
                    known_above
                    + ask["uncertain"]
                    + 3642 * (low["positive"] + low["uncertain"]) / 642
                ),
                (auto["positive"] + auto["uncertain"])
                / (known_above + auto["uncertain"] + 3642 * low["positive"] / 642),
            ],
            "candidate_recall_label_bounds": [
                known_above
                / (known_above + 3642 * (low["positive"] + low["uncertain"]) / 642),
                (known_above + auto["uncertain"] + ask["uncertain"])
                / (
                    known_above
                    + auto["uncertain"]
                    + ask["uncertain"]
                    + 3642 * low["positive"] / 642
                ),
            ],
            "ask_rate_population": 301 / 4000,
            "low_positive_rate_known": low_rate,
            "low_positive_rate_wilson95": low_wilson,
            "low_positive_rate_project_bootstrap95": low_cluster,
            "population_positive_estimate": positive_estimate,
        },
        "goldset_sha256": ret.digest(
            json.dumps(goldset, ensure_ascii=False, sort_keys=True).encode()
        ),
        "method_sha256": ret.digest(Path(__file__).read_bytes()),
    }
    ret.write_once(QUALITY / "goldset.json", goldset)
    ret.write_once(QUALITY / "summary.json", result)
    print(json.dumps(result, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare-third", "score"))
    args = parser.parse_args()
    if args.action == "prepare-third":
        prepare_third()
    else:
        score()


if __name__ == "__main__":
    main()
