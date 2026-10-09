"""봉인한 Astra 판정과 기존 Jev 응답을 검증하고 대조한다."""

from __future__ import annotations

import collections
import importlib.util
import json
import math
import random
import statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "retrospective", HERE / "retrospective.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("retrospective script unavailable")
ret = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ret)
OUT = ret.OUT
SEED = 38220261005
REPEATS = 10_000


def load_labels(group: Path) -> tuple[list[dict], dict[str, dict], dict]:
    seal = json.loads((group / "seal.json").read_text())
    for name, expected in seal["files"].items():
        if ret.digest((ret.ROOT / name).read_bytes()) != expected:
            raise RuntimeError(f"seal mismatch: {name}")
    cases = json.loads((group / "cases.json").read_text())
    jobs = ret.batches(cases)
    labels = {}
    raw = sorted((group / "raw").glob("*.json"))
    missing = sorted(set(range(len(jobs))) - {int(path.stem) for path in raw})
    invalid = []
    bad_evidence = []
    for path in raw:
        index = int(path.stem)
        record = json.loads(path.read_text())
        ids = [row["id"] for row in jobs[index]]
        values, meta = ret.parse_events(record.get("stdout", ""), ids)
        valid = (
            record["status"] == "ok"
            and record["model"] == ret.MODEL
            and record["ids"] == ids
            and record["prompt_sha256"] == ret.digest(ret.prompt(jobs[index]).encode())
            and record["values"] == values
            and values is not None
            and not meta["tool_events"]
        )
        if not valid:
            invalid.append(index)
            continue
        for case, value in zip(jobs[index], values):
            allowed = {"target"} | {
                f"future:{row['index']}"
                for row in ret.judge_view(case)["subsequent_conversation"]
            }
            if not value["evidence"] or any(
                ref not in allowed for ref in value["evidence"]
            ):
                bad_evidence.append(value["id"])
            if value["id"] in labels:
                raise RuntimeError("duplicate label")
            labels[value["id"]] = value
    return (
        cases,
        labels,
        {
            "batches": len(jobs),
            "raw": len(raw),
            "missing": missing,
            "invalid": invalid,
            "bad_evidence": bad_evidence,
        },
    )


def wilson(successes: int, total: int, alpha: float) -> list[float] | None:
    if total == 0:
        return None
    z = statistics.NormalDist().inv_cdf(1 - alpha / 2)
    p = successes / total
    denominator = 1 + z * z / total
    center = (p + z * z / (2 * total)) / denominator
    margin = (
        z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denominator
    )
    return [max(0.0, center - margin), min(1.0, center + margin)]


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = fraction * (len(ordered) - 1)
    lower = math.floor(position)
    upper = math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def counts(rows: list[dict]) -> dict[str, int]:
    known = [row for row in rows if row["gold"] != "uncertain"]
    return {
        "total": len(rows),
        "known": len(known),
        "uncertain": len(rows) - len(known),
        "auto": sum(row["jev_p"] >= 0.77 for row in known),
        "auto_all": sum(row["jev_p"] >= 0.77 for row in rows),
        "true_positive": sum(
            row["jev_p"] >= 0.77 and row["gold"] == "constraint" for row in rows
        ),
        "positive": sum(row["gold"] == "constraint" for row in rows),
        "candidate_true_positive": sum(
            row["jev_p"] >= 0.43 and row["gold"] == "constraint" for row in rows
        ),
        "ask": sum(0.43 <= row["jev_p"] < 0.77 for row in rows),
        "uncertain_below_auto": sum(
            row["gold"] == "uncertain" and row["jev_p"] < 0.77 for row in rows
        ),
        "uncertain_below_candidate": sum(
            row["gold"] == "uncertain" and row["jev_p"] < 0.43 for row in rows
        ),
    }


def cluster_intervals(rows: list[dict]) -> dict[str, dict]:
    by_project = collections.defaultdict(list)
    for row in rows:
        by_project[row["project_id"]].append(row)
    groups = [counts(group) for group in by_project.values()]
    rng = random.Random(SEED)
    samples = {
        name: []
        for name in ("precision", "auto_recall", "candidate_recall", "ask_rate")
    }
    zero = collections.Counter()
    for _ in range(REPEATS):
        selected = rng.choices(groups, k=len(groups))
        merged = {key: sum(group[key] for group in selected) for key in groups[0]}
        metrics = {
            "precision": (merged["true_positive"], merged["auto"]),
            "auto_recall": (merged["true_positive"], merged["positive"]),
            "candidate_recall": (merged["candidate_true_positive"], merged["positive"]),
            "ask_rate": (merged["ask"], merged["total"]),
        }
        for name, (numerator, denominator) in metrics.items():
            if denominator:
                samples[name].append(numerator / denominator)
            else:
                zero[name] += 1
    alpha = {
        "precision": 0.0125,
        "auto_recall": 0.0125,
        "candidate_recall": 0.00625,
        "ask_rate": 0.00625,
    }
    return {
        name: {
            "interval": [
                percentile(values, alpha[name] / 2),
                percentile(values, 1 - alpha[name] / 2),
            ]
            if values
            else None,
            "zero_denominator_resamples": zero[name],
        }
        for name, values in samples.items()
    }


def main() -> None:
    groups = [OUT, OUT / "retry", OUT / "recovered"]
    loaded = [load_labels(group) for group in groups]
    if loaded[0][2]["missing"] != [25, 26, 27, 28]:
        raise RuntimeError("unexpected original missing batches")
    if any(audit["missing"] or audit["invalid"] for _, _, audit in loaded[1:]):
        raise RuntimeError("incomplete recovered labels")
    cases = loaded[0][0] + loaded[2][0]
    labels = {}
    for _, group_labels, _ in loaded:
        if labels.keys() & group_labels.keys():
            raise RuntimeError("duplicate case across groups")
        labels.update(group_labels)
    if len(cases) != 4000 or {row["id"] for row in cases} != labels.keys():
        raise RuntimeError("missing or duplicate case labels")
    jev = {}
    for path in (ret.PRIVATE / "raw").glob("*.json"):
        record = json.loads(path.read_text())
        value = (record.get("probabilities") or {}).get("korean")
        if type(value) in (int, float) and math.isfinite(value):
            jev[record["sample_id"]] = value
    if {row["id"] for row in cases} != jev.keys():
        raise RuntimeError("missing Jev response")
    rows = [
        {
            "id": case["id"],
            "project_id": case["project_id"],
            "provider": case["provider"],
            "gold": labels[case["id"]]["label"],
            "category": labels[case["id"]]["category"],
            "reason": labels[case["id"]]["reason"],
            "evidence": labels[case["id"]]["evidence"],
            "jev_p": jev[case["id"]],
            "future_omitted_chars": case["future_coverage"]["omitted_chars"],
        }
        for case in cases
    ]
    result = counts(rows)
    metrics = {
        "precision": (result["true_positive"], result["auto"], 0.0125),
        "auto_recall": (result["true_positive"], result["positive"], 0.0125),
        "candidate_recall": (
            result["candidate_true_positive"],
            result["positive"],
            0.00625,
        ),
        "ask_rate": (result["ask"], result["total"], 0.00625),
    }
    summary = {
        "ts_utc": ret.now(),
        "selected": 4000,
        "source_matched_original": len(loaded[0][0]),
        "source_recovered": len(loaded[2][0]),
        "labels": dict(collections.Counter(row["gold"] for row in rows)),
        "categories": dict(collections.Counter(row["category"] for row in rows)),
        "counts": result,
        "metrics": {
            name: {
                "value": numerator / denominator if denominator else None,
                "wilson_95": wilson(numerator, denominator, 0.05),
                "wilson_adjusted": wilson(numerator, denominator, alpha),
            }
            for name, (numerator, denominator, alpha) in metrics.items()
        },
        "cluster_bootstrap_adjusted": cluster_intervals(rows),
        "conservative": {
            "precision": result["true_positive"] / result["auto_all"]
            if result["auto_all"]
            else None,
            "auto_recall": result["true_positive"]
            / (result["positive"] + result["uncertain_below_auto"])
            if result["positive"] + result["uncertain_below_auto"]
            else None,
            "candidate_recall": result["candidate_true_positive"]
            / (result["positive"] + result["uncertain_below_candidate"])
            if result["positive"] + result["uncertain_below_candidate"]
            else None,
        },
        "truncated_future": sum(row["future_omitted_chars"] > 0 for row in rows),
        "groups": {
            str(group.relative_to(OUT)) if group != OUT else "original": audit
            for group, (_, _, audit) in zip(groups, loaded)
        },
        "score_rows_sha256": ret.digest(
            json.dumps(rows, ensure_ascii=False, sort_keys=True).encode()
        ),
        "method_sha256": ret.digest(Path(__file__).read_bytes()),
    }
    (OUT / "full-score-rows.json").write_text(
        json.dumps(rows, ensure_ascii=False, indent=2) + "\n"
    )
    (OUT / "full-summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps(summary, ensure_ascii=False))


if __name__ == "__main__":
    main()
