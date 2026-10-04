"""봉인된 정답과 응답을 정규화하고 집계만 공개한다."""

from __future__ import annotations

import csv
import json
import sys
from collections import Counter

from metrics import measured_rate, paired, rate, timing
from protocol import CANDIDATES, DEFAULT, REFERENCE, normalize_choice
from runtime import (
    PRIVATE,
    PUBLIC,
    parse_json,
    read,
    response_text,
    rows,
    write,
)


def canonical(model: str) -> str | None:
    return "claude/default" if model == "claude/opus" else model


def accepts(model: str, label: dict) -> bool:
    return canonical(model) in {canonical(m) for m in label["acceptable"]}


def cost_of(record: dict, meta: dict) -> float | None:
    if not isinstance(meta, dict):
        return None
    if record["kind"] == "claude":
        return meta.get("total_cost_usd")
    usage = meta.get("usage", {})
    if not isinstance(usage, dict) or "input_tokens" not in usage:
        return None
    if record["kind"] == "jev":
        return usage["input_tokens"] * 0.042 / 1e6
    prices = {"gpt-6-astra": (10, 1, 50), "gpt-6-luna": (0.1, 0.01, 0.5)}
    if record.get("model") not in prices:
        return None
    uncached, cached, output = prices[record["model"]]
    cache_tokens = usage.get("cached_input_tokens", 0)
    return (
        (usage["input_tokens"] - cache_tokens) * uncached
        + cache_tokens * cached
        + usage.get("output_tokens", 0) * output
    ) / 1e6


def normalize_record(
    case: dict,
    label: dict | None,
    lane: str,
    repeat: int,
    reserved: set[str] | None = None,
) -> dict:
    trial = lane + "-" + case["sample_id"] + "-" + str(repeat)
    path = PRIVATE / "raw" / (trial + ".json")
    record = (
        read(path)
        if path.exists()
        else dict(
            kind="jev" if lane == "jev" else REFERENCE[lane][0],
            status="incomplete" if trial in (reserved or set()) else "not_run",
        )
    )
    if lane == "jev":
        envelope = record.get("response", {})
        if not isinstance(envelope, dict):
            envelope = {}
        meta = dict(usage=envelope.get("usage", {}))
        choice = normalize_choice(envelope) if record.get("status") == "ok" else None
    else:
        text, meta = response_text(record)
        choice = None if meta.get("tool_events") else normalize_choice(parse_json(text))
    selected = choice["selected"] if choice else None
    effective = choice["effective"] if choice else DEFAULT
    row = dict(
        run_id=read(PRIVATE / "design-seal.json")["commit"][:7],
        trial_id=trial,
        ts_utc=record.get("ts_utc"),
        sample_id=case["sample_id"],
        project_id=case["project_id"],
        category=case["category"],
        source=case["source"],
        condition=lane,
        repeat=repeat,
        selected=selected,
        effective=effective,
        valid=choice is not None,
        status=record["status"],
        fallback=choice["fallback"] if choice else True,
        low_confidence=choice["low_confidence"] if choice else False,
        confidence=choice["confidence"] if choice else None,
        latency_s=record.get("latency_s"),
        cost_usd=cost_of(record, meta),
        switch=effective.startswith("claude/"),
        best_match=None,
        raw_hit=None,
        effective_hit=None,
        manual_hit=None,
        switch_hit=None,
        signal=label["signal"] if label else None,
    )
    if label and label["status"] == "judged" and record["status"] != "not_run":
        row.update(
            best_match=canonical(selected) == canonical(label["best"]),
            raw_hit=accepts(selected, label),
            effective_hit=accepts(effective, label),
            manual_hit=accepts(DEFAULT, label),
        )
        row["switch_hit"] = row["effective_hit"] if row["switch"] else None
    return row


def process() -> list[dict]:
    labels = {r["sample_id"]: r["final"] for r in read(PRIVATE / "labels.json")}
    data = []
    reserved = {r["trial_id"] for r in rows(PRIVATE / "calls.jsonl")}
    for case in read(PRIVATE / "samples.json"):
        for lane in ["jev", *REFERENCE]:
            for repeat in range(1, 4 if lane == "jev" else 2):
                data.append(
                    normalize_record(
                        case, labels[case["sample_id"]], lane, repeat, reserved
                    )
                )
    path = PRIVATE / "processed/observations.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in data))
    return data


def summarize_condition(all_rows: list) -> dict:
    attempted = [r for r in all_rows if r["status"] != "not_run"]
    judged = [r for r in all_rows if r["effective_hit"] is not None]
    valid = [r for r in judged if r["valid"]]
    costs = [r["cost_usd"] for r in all_rows if r["cost_usd"] is not None]
    return dict(
        best=measured_rate(judged, "best_match"),
        raw=measured_rate(judged, "raw_hit"),
        effective=measured_rate(judged, "effective_hit"),
        manual=measured_rate(judged, "manual_hit"),
        paired=paired(judged),
        switch=measured_rate(judged, "switch"),
        switch_hit=measured_rate(judged, "switch_hit"),
        fallback=rate([r["fallback"] for r in attempted]),
        low_confidence=rate([r["low_confidence"] for r in attempted]),
        invalid=sum(not r["valid"] for r in attempted),
        not_run=len(all_rows) - len(attempted),
        attempted=len(attempted),
        statuses=dict(Counter(r["status"] for r in all_rows)),
        success_conditional_raw=rate([r["raw_hit"] for r in valid]),
        latency=timing(
            [r["latency_s"] for r in all_rows if r["latency_s"] is not None]
        ),
        known_cost_usd=sum(costs),
        unknown_cost_calls=len(attempted) - len(costs),
        mean_known_cost_usd=sum(costs) / len(costs) if costs else None,
        choices=dict(Counter(r["selected"] or "invalid" for r in all_rows)),
    )


def adjudication() -> dict:
    labels = read(PRIVATE / "labels.json")
    exclusions = Counter()
    for r in labels:
        if r["final"] is None:
            exclusions["no_majority"] = exclusions["no_majority"] + 1
        elif r["final"]["status"] == "uncertain":
            exclusions["uncertain"] = exclusions["uncertain"] + 1
    comparable = [r for r in labels if all(r["ballots"][:2])]
    return dict(
        total=len(labels),
        first_complete_agreement=rate([r["first_agree"] for r in labels]),
        first_best_agreement=rate(
            [
                canonical(r["ballots"][0]["best"]) == canonical(r["ballots"][1]["best"])
                for r in comparable
            ]
        ),
        first_set_agreement=rate(
            [
                r["ballots"][0]["acceptable"] == r["ballots"][1]["acceptable"]
                for r in comparable
            ]
        ),
        third_calls=sum(len(r["ballots"]) == 3 for r in labels),
        exclusions=dict(exclusions),
        analyzed=len(labels) - sum(exclusions.values()),
        invalid_ballots=sum(v is None for r in labels for v in r["ballots"]),
        signal_counts=dict(Counter(r["final"]["signal"] for r in labels if r["final"])),
        final_best=dict(
            Counter(
                r["final"]["best"]
                for r in labels
                if r["final"] and r["final"]["status"] == "judged"
            )
        ),
    )


def verdict(jev: dict) -> dict:
    delta = jev["paired"]["cluster95"]
    h1 = (
        "보류"
        if delta is None
        else "채택"
        if delta[0] >= -0.05
        else "기각"
        if delta[1] < -0.05
        else "보류"
    )
    switch = jev["switch_hit"]
    w, c = switch["wilson95"], switch["cluster95"]
    h2 = (
        "보류"
        if w is None or c is None
        else "채택"
        if min(w[0], c[0]) >= 0.8
        else "기각"
        if w[1] < 0.8
        else "보류"
    )
    recommendation = (
        "오토 기본 유지 권장"
        if h1 == h2 == "채택"
        else "매뉴얼 기본 또는 오토 선택적 사용 권장"
        if "기각" in (h1, h2)
        else "기본값 권고 보류, 독립 표본 추가 필요"
    )
    return dict(H1=h1, H2=h2, recommendation=recommendation)


def sensitivity(data: list) -> dict:
    labels = {r["sample_id"]: r["final"] for r in read(PRIVATE / "labels.json")}
    base = [
        r
        for r in data
        if r["condition"] == "jev"
        and r["repeat"] == 1
        and r["effective_hit"] is not None
    ]
    results = {}
    for candidate in CANDIDATES:
        adjusted = []
        for row in base:
            label = labels[row["sample_id"]]
            effective = candidate if row["fallback"] else row["selected"]
            adjusted.append(
                dict(
                    row,
                    effective_hit=accepts(effective, label),
                    manual_hit=accepts(candidate, label),
                )
            )
        results[candidate] = dict(
            manual=rate([r["manual_hit"] for r in adjusted]),
            auto=rate([r["effective_hit"] for r in adjusted]),
            paired=paired(adjusted),
        )
    return results


# cost: time O(b*p+n), io n receipts; vars: b = bootstrap draws, p = projects, n = calls; basis: estimate
def analyze(data: list) -> None:
    conditions = {}
    for lane in ["jev", *REFERENCE]:
        for repeat in range(1, 4 if lane == "jev" else 2):
            group = [
                r for r in data if r["condition"] == lane and r["repeat"] == repeat
            ]
            conditions[lane + "-" + str(repeat)] = summarize_condition(group)
    by_sample = {}
    for row in data:
        if row["condition"] == "jev":
            by_sample.setdefault(row["sample_id"], []).append(row)
    agreement = rate(
        [
            len(g) == 3
            and all(r["valid"] for r in g)
            and len({r["selected"] for r in g}) == 1
            for g in by_sample.values()
        ]
    )
    ledger = rows(PRIVATE / "calls.jsonl")
    raw_costs = []
    for reservation in ledger:
        path = PRIVATE / "raw" / (reservation["trial_id"] + ".json")
        if not path.exists():
            raw_costs.append(None)
            continue
        record = read(path)
        if record["kind"] == "jev":
            meta = record.get("response", {})
        else:
            _, meta = response_text(record)
        raw_costs.append(cost_of(record, meta))
    summary = dict(
        sampling=read(PRIVATE / "sampling.json"),
        adjudication=adjudication(),
        conditions=conditions,
        jev_three_way_agreement=agreement,
        manual_sensitivity=sensitivity(data),
        verdict=verdict(conditions["jev-1"]),
        calls=dict(Counter(r["kind"] for r in ledger)),
        call_limits={"codex": 500, "claude": 300, "jev": 400},
        incomplete_reserved_calls=sum(
            not (PRIVATE / "raw" / (r["trial_id"] + ".json")).exists() for r in ledger
        ),
        all_known_cost_usd=sum(c for c in raw_costs if c is not None),
        all_unknown_cost_calls=sum(c is None for c in raw_costs),
        design_commit=read(PRIVATE / "design-seal.json")["commit"],
        seed=338100,
        confidence_threshold=0.6,
        noninferiority_margin_pp=5,
        switch_target_percent=80,
        planned_n=100,
        source_audit=read(PRIVATE / "source-audit.json"),
        parser_correction=read(PRIVATE / "parser-correction.json"),
        pre_audit_reserved_calls=read(PRIVATE / "pre-audit-calls.json")[
            "reserved_gold"
        ],
        rejected_source_samples=len(
            read(PRIVATE / "rejected-census/source-audit.json")["excluded_sample_ids"]
        ),
    )
    primary = [r for r in data if r["condition"] == "jev" and r["repeat"] == 1]
    for field in ("category", "source", "signal"):
        summary["by_" + field] = {
            value: summarize_condition([r for r in primary if r[field] == value])
            for value in sorted({r[field] for r in primary if r[field] is not None})
        }
    write(PUBLIC / "results/summary.json", summary)
    with (PUBLIC / "results/conditions.csv").open("w") as stream:
        writer = csv.writer(stream)
        writer.writerow(
            [
                "condition",
                "n",
                "best_match",
                "raw_hit",
                "effective_hit",
                "manual_hit",
                "delta",
                "switch_n",
                "switch_hit",
                "fallback",
                "median_s",
                "known_cost_usd",
            ]
        )
        for name, c in conditions.items():
            writer.writerow(
                [
                    name,
                    c["raw"]["n"],
                    c["best"]["rate"],
                    c["raw"]["rate"],
                    c["effective"]["rate"],
                    c["manual"]["rate"],
                    c["paired"]["delta"],
                    c["switch_hit"]["n"],
                    c["switch_hit"]["rate"],
                    c["fallback"]["rate"],
                    c["latency"]["median_s"],
                    c["known_cost_usd"],
                ]
            )
    print(
        json.dumps(
            dict(
                adjudication=summary["adjudication"],
                calls=summary["calls"],
                verdict=summary["verdict"],
            ),
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    data = process()
    if len(sys.argv) < 2 or sys.argv[1] != "process":
        analyze(data)
