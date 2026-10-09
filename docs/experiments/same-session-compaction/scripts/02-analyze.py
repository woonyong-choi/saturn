"""원본 실행 로그를 검증하고 대응 축약 결과를 재계산한다."""

from __future__ import annotations

import csv
import hashlib
import io
import json
import math
import random
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
RAW = ROOT / ".local/experiments/same-session-compaction/formal"
HERE = Path(__file__).resolve().parents[1]
ARMS = ("full", "native", "fast", "packet")
Json = dict[str, Any]


def read_events(path: Path) -> list[Json]:
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def result_of(events: list[Json]) -> Json:
    results = [event for event in events if event.get("type") == "result"]
    if len(results) != 1 or results[0].get("is_error"):
        raise ValueError("missing, duplicate or failed result event")
    return results[0]


def tool_calls(events: list[Json]) -> list[Json]:
    return [
        block
        for event in events
        if event.get("type") == "assistant"
        for block in event.get("message", {}).get("content", [])
        if isinstance(block, dict) and block.get("type") == "tool_use"
    ]


def verify_base(row: Json, streams: dict[str, list[Json]]) -> str:
    calls = []
    for turn in range(1, 5):
        events = streams[f"base_{turn}"]
        if result_of(events)["session_id"] != row["base_session"]:
            raise ValueError("base session changed")
        calls.extend(tool_calls(events))
    reads = [call for call in calls if call.get("name") == "Read"]
    filenames = {Path(call["input"]["file_path"]).name for call in reads}
    expected = {f"case-{index:02}.txt" for index in range(1, 9)}
    if filenames != expected or len(reads) != row["reads"]:
        raise ValueError("initial Read coverage mismatch")
    assistants = [e for e in streams["base_4"] if e.get("type") == "assistant"]
    return assistants[-1]["uuid"]


def verify_arm(row: Json, arm: str, streams: dict[str, list[Json]]) -> None:
    events = streams[f"{arm}_answer"]
    result = result_of(events)
    recorded = row["arms"][arm]
    if result["session_id"] != recorded["session_id"]:
        raise ValueError(f"answer session mismatch: {arm}")
    if tool_calls(events):
        raise ValueError(f"unexpected answer tool call: {arm}")
    if result.get("result", "") != recorded["text"]:
        raise ValueError(f"answer text mismatch: {arm}")
    if (row["answer"] in result.get("result", "")) != recorded["exact"]:
        raise ValueError(f"exact score mismatch: {arm}")
    if result["total_cost_usd"] != row["costs"][f"{arm}_answer"]:
        raise ValueError(f"cost mismatch: {arm}")


def verify_compaction(row: Json, streams: dict[str, list[Json]], parent: str) -> Json:
    boundaries = {}
    for arm in ("fast", "native"):
        events = streams[f"{arm}_compact"]
        result = result_of(events)
        if result["session_id"] != row["arms"][arm]["session_id"]:
            raise ValueError("compaction session mismatch")
        found = [e for e in events if e.get("subtype") == "compact_boundary"]
        if len(found) != 1 or found[0].get("logical_parent_uuid") != parent:
            raise ValueError("fork ancestry or compact boundary mismatch")
        boundaries[arm] = found[0]["compact_metadata"]
    if any(e.get("plugin") == "fast-jev-compaction" for e in streams["native_compact"]):
        raise ValueError("native compaction unexpectedly loaded fast-jev hook")
    logs = [
        event.get("text", "")
        for event in streams["fast_compact"]
        if event.get("subtype") == "ui_log"
        and event.get("plugin") == "fast-jev-compaction"
    ]
    applied = any("no summary" in line for line in logs)
    dropped = sum(
        int(match.group(1))
        for line in logs
        for match in re.finditer(r"(\d+) call_dropped", line)
    )
    return {"boundaries": boundaries, "hook_applied": applied, "dropped": dropped}


def audit_row(row: Json) -> Json:
    folder = RAW / f"scenario-{row['index']:03}"
    names = [f"base_{turn}" for turn in range(1, 5)]
    names += [f"{arm}_answer" for arm in ARMS] + ["fast_compact", "native_compact"]
    streams = {name: read_events(folder / f"{name}.jsonl") for name in names}
    parent = verify_base(row, streams)
    if len({row["arms"][arm]["session_id"] for arm in ARMS}) != 4:
        raise ValueError("condition sessions are not distinct")
    if row["arms"]["full"]["session_id"] != row["base_session"]:
        raise ValueError("full condition did not resume base session")
    for arm in ARMS:
        verify_arm(row, arm, streams)
    fixture = (folder / f"case-{row['target']:02}.txt").read_text()
    if row["answer"] not in fixture:
        raise ValueError("answer does not match source fixture")
    packet = json.loads((folder / "packet.json").read_text())
    if packet["tokens"] != row["arms"]["packet"]["packet_tokens"]:
        raise ValueError("packet token mismatch")
    evidence = verify_compaction(row, streams, parent)
    evidence["target_in_packet"] = row["answer"] in packet["packet"]
    evidence["packet_included_calls"] = len(packet["included"])
    evidence["models"] = sorted(
        {
            model
            for events in streams.values()
            for model in result_of(events).get("modelUsage", {})
        }
    )
    evidence["answer_input_tokens"] = {
        arm: sum(
            result_of(streams[f"{arm}_answer"])["usage"].get(key, 0)
            for key in (
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            )
        )
        for arm in ARMS
    }
    evidence["elapsed_ms"] = {
        arm: result_of(streams[f"{arm}_answer"])["duration_ms"]
        + (
            result_of(streams[f"{arm}_compact"])["duration_ms"]
            if arm in ("fast", "native")
            else 0
        )
        for arm in ARMS
    }
    return evidence


def wilson(hits: int, n: int) -> list[float] | None:
    if not n:
        return None
    z = 1.959963984540054
    rate = hits / n
    denominator = 1 + z * z / n
    center = (rate + z * z / (2 * n)) / denominator
    half = z * math.sqrt(rate * (1 - rate) / n + z * z / (4 * n * n)) / denominator
    return [max(0.0, center - half), min(1.0, center + half)]


def paired(rows: list[Json], arms: tuple[str, str], seed: int) -> Json:
    a, b = arms
    pairs = [(int(r["arms"][a]["exact"]), int(r["arms"][b]["exact"])) for r in rows]
    if not pairs:
        return {"n": 0, "difference": None, "ci95": None, "a_only": 0, "b_only": 0}
    n = len(pairs)
    differences = [x - y for x, y in pairs]
    rng = random.Random(seed)
    samples = sorted(
        sum(rng.choice(differences) for _ in range(n)) / n for _ in range(10000)
    )
    return {
        "n": n,
        "difference": sum(differences) / n,
        "ci95": [samples[249], samples[9749]],
        "a_only": sum(x and not y for x, y in pairs),
        "b_only": sum(y and not x for x, y in pairs),
    }


def numeric_summary(values: list[float]) -> Json:
    return {
        "sum": round(sum(values), 6),
        "mean": round(sum(values) / len(values), 6),
        "min": min(values),
        "max": max(values),
    }


def summarize(rows: list[Json], audits: list[Json]) -> Json:
    complete = [row for row in rows if "error" not in row]
    summary: Json = {
        "attempted": len(rows),
        "complete": len(complete),
        "planned": 48,
        "errors": [
            {"index": r["index"], "error": r["error"]} for r in rows if "error" in r
        ],
        "conditions": {},
        "paired": {},
        "claude_incremental_cost": {},
        "position": {},
        "compaction": {},
        "elapsed_ms": {},
        "answer_input_tokens": {},
        "hook_applied": sum(a["hook_applied"] for a in audits),
        "packet_targets_present": sum(a["target_in_packet"] for a in audits),
        "packet_ready_only": sum(
            r["arms"]["packet"]["text"].strip() == "Ready" for r in complete
        ),
        "packet_included_calls": numeric_summary(
            [a["packet_included_calls"] for a in audits]
        ),
        "fast_dropped_calls": sum(a["dropped"] for a in audits),
        "total_read_calls": sum(r["reads"] for r in complete),
        "models": sorted({model for audit in audits for model in audit["models"]}),
        "audit": {
            "paired_ancestry_verified": len(audits),
            "answer_tool_calls": 0,
            "exact_scores_recomputed": len(complete) * len(ARMS),
        },
    }
    for arm in ARMS:
        hits = sum(r["arms"][arm]["exact"] for r in complete)
        summary["conditions"][arm] = {
            "hits": hits,
            "n": len(complete),
            "rate": hits / len(complete),
            "wilson95": wilson(hits, len(complete)),
        }
        costs = [
            r["costs"][f"{arm}_answer"]
            - (0 if arm == "packet" else r["costs"]["base_4"])
            for r in complete
        ]
        summary["claude_incremental_cost"][arm] = numeric_summary(costs)
        summary["elapsed_ms"][arm] = numeric_summary(
            [a["elapsed_ms"][arm] for a in audits]
        )
        summary["answer_input_tokens"][arm] = numeric_summary(
            [a["answer_input_tokens"][arm] for a in audits]
        )
    for target in range(1, 9):
        selected = [r for r in complete if r["target"] == target]
        summary["position"][str(target)] = {
            "n": len(selected),
            **{arm: sum(r["arms"][arm]["exact"] for r in selected) for arm in ARMS},
        }
    for arm in ("fast", "native"):
        summary["compaction"][arm] = {
            field: numeric_summary([a["boundaries"][arm][field] for a in audits])
            for field in ("pre_tokens", "post_tokens", "duration_ms")
        }
    summary["paired"]["fast_minus_native"] = paired(
        complete, ("fast", "native"), 20261005
    )
    summary["paired"]["packet_minus_full"] = paired(
        complete, ("packet", "full"), 20261006
    )
    applied_rows = [
        row for row, audit in zip(complete, audits) if audit["hook_applied"]
    ]
    summary["paired"]["fast_hook_only_minus_native"] = paired(
        applied_rows, ("fast", "native"), 20261005
    )
    h1 = summary["paired"]["fast_minus_native"]
    h2 = summary["paired"]["packet_minus_full"]
    summary["hypotheses"] = {
        "H1": {
            "accepted": abs(h1["difference"]) >= 0.20
            and (h1["ci95"][0] > 0 or h1["ci95"][1] < 0)
        },
        "H2": {"accepted": h2["ci95"][0] > -0.20},
    }
    summary["hook_fallbacks"] = len(audits) - summary["hook_applied"]
    summary["fast_retained_calls"] = (
        summary["total_read_calls"] - summary["fast_dropped_calls"]
    )
    summary["packet_tokens"] = numeric_summary(
        [r["arms"]["packet"]["packet_tokens"] for r in complete]
    )
    return summary


def write_checked(path: Path, content: str, *, is_verify: bool) -> None:
    if is_verify:
        if not path.exists() or path.read_bytes() != content.encode("utf-8"):
            raise ValueError(f"saved artifact differs from recomputation: {path.name}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def write_table(path: Path, rows: list[Json], *, is_verify: bool) -> None:
    output = io.StringIO(newline="")
    writer = csv.DictWriter(output, fieldnames=list(rows[0]), lineterminator="\r\n")
    writer.writeheader()
    writer.writerows(rows)
    write_checked(path, output.getvalue(), is_verify=is_verify)


def main() -> None:
    rows = read_events(RAW / "results.jsonl")
    if [r["index"] for r in rows] != list(range(1, 49)):
        raise ValueError("formal collection is incomplete or indices are not unique")
    complete = [row for row in rows if "error" not in row]
    if len(complete) != 48:
        raise ValueError("failed trials require conservative and success-only analyses")
    session_ids = [row["arms"][arm]["session_id"] for row in complete for arm in ARMS]
    if len(set(session_ids)) != len(session_ids):
        raise ValueError("session reused across scenarios")
    audits = [audit_row(row) for row in complete]
    summary = summarize(rows, audits)
    summary["environment"] = json.loads((HERE / "env.json").read_text())
    summary["bootstrap_replicates"] = 10000
    summary["acceptance_margin"] = 0.20
    manifest = []
    for path in sorted(RAW.rglob("*")):
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"apikey_" in data:
            raise ValueError("secret-like value in raw data")
        manifest.append(f"{hashlib.sha256(data).hexdigest()}  {path.relative_to(RAW)}")
    manifest_text = "\n".join(manifest) + "\n"
    summary["raw_sha256"] = hashlib.sha256(
        (RAW / "results.jsonl").read_bytes()
    ).hexdigest()
    summary["manifest_sha256"] = hashlib.sha256(manifest_text.encode()).hexdigest()
    summary["raw_file_count"] = len(manifest)
    content = json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    is_verify = "--verify" in sys.argv
    write_checked(HERE / "data/SHA256SUMS", manifest_text, is_verify=is_verify)
    write_checked(HERE / "results/summary.json", content, is_verify=is_verify)
    table = [
        {
            "scenario": row["index"],
            "target": row["target"],
            "condition": arm,
            "exact": int(row["arms"][arm]["exact"]),
            "input_tokens": audit["answer_input_tokens"][arm],
            "cost_usd": round(
                row["costs"][f"{arm}_answer"]
                - (0 if arm == "packet" else row["costs"]["base_4"]),
                8,
            ),
        }
        for row, audit in zip(complete, audits)
        for arm in ARMS
    ]
    write_table(HERE / "results/tables/resumed.csv", table, is_verify=is_verify)
    print(
        json.dumps(
            {
                "complete": len(complete),
                "verified": is_verify,
                "hits": {arm: summary["conditions"][arm]["hits"] for arm in ARMS},
            }
        )
    )


if __name__ == "__main__":
    main()
