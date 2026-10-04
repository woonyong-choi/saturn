"""저장 원응답에서 라벨과 관측 행을 재생성한다."""

from __future__ import annotations

import json
import math
from collections import Counter

from protocol import parse
from runtime import PRIVATE, read, response_text, rows, write


def read_cli_values(record: dict) -> list[dict]:
    data = json.loads(record["prompt"].split("DATA:\n")[-1])
    phase = "query" if record["trial_id"].startswith("query-") else "gold"
    values = parse(record, [c["id"] for c in data], phase)
    return [
        dict(id=c["id"], value=values[i] if values else None)
        for i, c in enumerate(data)
    ]


def cost(record: dict) -> float | None:
    if record["kind"] == "jev":
        usage = record.get("response", {}).get("usage", {})
        return usage["input_tokens"] * 0.042 / 1e6 if "input_tokens" in usage else None
    try:
        _, envelope = response_text(record)
    except (ValueError, TypeError, KeyError):
        return None
    usage = envelope.get("usage", {})
    if "input_tokens" not in usage or "output_tokens" not in usage:
        return None
    price = 10 if record["model"] == "gpt-6-astra" else 2
    cached, written = (
        usage.get("cached_input_tokens", 0),
        usage.get("cache_write_input_tokens", 0),
    )
    ordinary = usage["input_tokens"] - cached - written
    return (
        ordinary * price
        + cached * price * 0.1
        + written * price * 1.25
        + usage["output_tokens"] * price * 5
    ) / 1e6


def jev_value(record: dict) -> tuple:
    response = record.get("response", {})
    answers = response.get("answers", {}) if isinstance(response, dict) else {}
    expected = set(record["request"]["questions"])
    if (
        record.get("status") != "ok"
        or response.get("model") != "jev-1.13.0"
        or set(answers) != expected
    ):
        return None, None
    values = []
    for key in sorted(expected):
        answer = answers[key]
        p = answer.get("noul") if isinstance(answer, dict) else None
        if (
            type(p) not in (float, int)
            or not math.isfinite(p)
            or not 0 <= p <= 1
            or answer.get("type") != "noul"
        ):
            return None, None
        values.append(p)
    p = answers["is_constraint"]["noul"]
    task = answers.get("task_only", {}).get("noul")
    return (0 if task is not None and task >= 0.5 else p), task


def rebuild_gold(records: list[dict], cases: list[dict]) -> list[dict]:
    votes = {}
    for record in records:
        phase = "-".join(record["trial_id"].split("-")[:2])
        if phase not in ("gold-sol", "gold-astra", "third-astra"):
            continue
        for row in read_cli_values(record):
            votes[(row["id"], phase)] = row["value"]
    result = []
    for case in cases:
        sid = case["sample_id"]
        judgments = [votes.get((sid, p)) for p in ("gold-sol", "gold-astra")]
        initially_agreed = bool(
            judgments[0]
            and judgments[1]
            and judgments[0]["label"] == judgments[1]["label"]
        )
        if not initially_agreed:
            judgments.append(votes.get((sid, "third-astra")))
        counts = Counter(v["label"] for v in judgments if v)
        winner, n = counts.most_common(1)[0] if counts else ("uncertain", 0)
        result.append(
            dict(
                sample_id=sid,
                label=winner if n >= 2 else "uncertain",
                judgments=judgments,
                initially_agreed=initially_agreed,
            )
        )
    return result


# cost: io saved experiment artifact reads and writes; basis: estimate
def main() -> None:
    cases = read(PRIVATE / "samples.json")
    records = [read(f) for f in sorted((PRIVATE / "raw").glob("*.json"))]
    gold = rebuild_gold(records, cases)
    if gold != read(PRIVATE / "gold.json"):
        raise RuntimeError("raw gold does not match sealed gold")
    by_id = {
        c["sample_id"]: dict(
            project_id=c["project_id"],
            input_kind=c["input_kind"],
            previously_seen=c["previously_seen"],
        )
        for c in cases
    }
    labels = {g["sample_id"]: g["label"] for g in gold}
    observations, accounting = [], []
    run_id = read(PRIVATE / "design-seal.json")["commit"][:7]
    for r in records:
        accounting.append(
            dict(
                trial_id=r["trial_id"],
                kind=r["kind"],
                status=r["status"],
                latency_s=r["latency_s"],
                cost_usd=cost(r),
            )
        )
        if r["kind"] == "jev":
            p, task = jev_value(r)
            values = [
                dict(id=r["sample_id"], value=dict(probability=p, task_only=task))
            ]
            condition, repeat = r["condition"], r["repeat"]
        elif r["trial_id"].startswith("query-"):
            values = read_cli_values(r)
            condition, repeat = "astra", 1
        else:
            continue
        for row in values:
            sid = row["id"]
            p = row["value"]["probability"] if row["value"] else None
            amount = cost(r)
            observations.append(
                dict(
                    run_id=run_id,
                    trial_id=r["trial_id"],
                    ts_utc=r["ts_utc"],
                    sample_id=sid,
                    condition=condition,
                    repeat=repeat,
                    gold=labels[sid],
                    probability=p,
                    task_only=row["value"].get("task_only") if row["value"] else None,
                    valid=p is not None,
                    latency_s=r["latency_s"],
                    amortized_latency_s=r["latency_s"] / len(values),
                    cost_usd=amount / len(values) if amount is not None else None,
                    **by_id[sid],
                )
            )
    observed = {(r["condition"], r["repeat"], r["sample_id"]) for r in observations}
    for case in cases:
        for condition, repeats in [("astra", (1,))] + [
            (c, (1, 2, 3)) for c in ("current", "scope", "scope_gate")
        ]:
            for repeat in repeats:
                sid = case["sample_id"]
                if (condition, repeat, sid) not in observed:
                    observations.append(
                        dict(
                            run_id=run_id,
                            trial_id=None,
                            ts_utc=None,
                            sample_id=sid,
                            condition=condition,
                            repeat=repeat,
                            gold=labels[sid],
                            probability=None,
                            task_only=None,
                            valid=False,
                            latency_s=None,
                            amortized_latency_s=None,
                            cost_usd=None,
                            **by_id[sid],
                        )
                    )
    observations.sort(key=lambda r: (r["condition"], r["repeat"], r["sample_id"]))
    (PRIVATE / "processed.jsonl").write_text(
        "".join(
            json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n"
            for r in observations
        )
    )
    write(PRIVATE / "accounting.json", accounting)
    print(
        json.dumps(
            dict(
                processed=len(observations),
                calls=len(rows(PRIVATE / "calls.jsonl")),
                complete=sum(r["valid"] for r in observations),
            )
        )
    )


if __name__ == "__main__":
    main()
