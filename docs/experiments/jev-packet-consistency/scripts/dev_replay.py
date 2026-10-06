"""Replay twelve sealed development packet requests without changing their source."""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from pathlib import Path
from typing import Any

import shared

REPEATS = 5
LOCK = Path(__file__).resolve().parent.parent / "eval" / "dev-source-lock.json"


def load_sources(source: Path) -> list[dict[str, Any]]:
    manifest = json.loads((source / "manifest.json").read_text(encoding="utf-8"))
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    if (
        hashlib.sha256((source / "manifest.json").read_bytes()).hexdigest()
        != lock["source_manifest_sha256"]
    ):
        raise ValueError("source manifest hash mismatch")
    if lock["repeats"] != REPEATS or lock["selection_wilson95_lower_required"] != 0.90:
        raise ValueError("development decision lock mismatch")
    if len(manifest["fixtures"]) != 12:
        raise ValueError("expected twelve sealed source fixtures")
    rows = []
    for index in range(12):
        name = f"t{index:02d}J"
        path = source / "raw" / f"{name}.json"
        if not path.exists():
            raise ValueError(f"missing sealed raw: {name}")
        raw = json.loads(path.read_text(encoding="utf-8"))
        judgments = [
            item
            for item in raw["db"]["judgments"]
            if "compact" in item.get("question_sets", []) and item.get("sent")
        ]
        if not judgments:
            raise ValueError(f"missing compact request: {name}")
        judgment = min(judgments, key=lambda item: item["id"])
        sent = judgment["sent"]
        request = json.loads(sent)
        if request.get("model") != lock["model_request"]:
            raise ValueError(f"request model differs from lock: {name}")
        if not isinstance(request.get("questions"), dict) or not request["questions"]:
            raise ValueError(f"invalid questions: {name}")
        packets = sorted(raw["db"]["packets"], key=lambda item: item["id"])
        if not packets:
            raise ValueError(f"missing packet: {name}")
        packet_id = packets[0]["id"]
        applied = sorted(
            item["ref_id"]
            for item in raw["db"]["items"]
            if item["packet_id"] == packet_id
            and item["zone"] == "Competing"
            and item["selector"] == "compact"
            and item["form"]
        )
        if not applied:
            raise ValueError(f"missing applied set: {name}")
        rows.append(
            {
                "id": name,
                "request": sent,
                "request_sha256": hashlib.sha256(sent.encode("utf-8")).hexdigest(),
                "k": len(applied),
                "applied_ids": applied,
                "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "recorded_response": judgment.get("received"),
            }
        )
    observed = [
        {
            "id": row["id"],
            "source_sha256": row["source_sha256"],
            "request_sha256": row["request_sha256"],
            "k": row["k"],
        }
        for row in rows
    ]
    if observed != lock["cases"]:
        raise ValueError("sealed development cases differ from lock")
    return rows


def collect(sources: list[dict[str, Any]], key_file: Path, output: Path) -> None:
    plan = [
        {
            "id": source["id"],
            "rep": rep,
            "request_text": source["request"],
            "metadata": {"source_sha256": source["source_sha256"]},
        }
        for source in sources
        for rep in range(1, REPEATS + 1)
    ]
    random.Random(5447).shuffle(plan)
    shared.collect_trials(plan, ("id", "rep"), key_file, output)


def analyze(sources: list[dict[str, Any]], output: Path) -> dict[str, Any]:
    starts, previous = shared.ledger(output, ("id", "rep"))
    details = []
    for source in sources:
        selections = []
        statuses = []
        for rep in range(1, REPEATS + 1):
            row = previous.get((source["id"], rep))
            if (
                row is None
                or row["request_sha256"] != source["request_sha256"]
                or row["source_sha256"] != source["source_sha256"]
            ):
                statuses.append("missing_or_hash_mismatch")
                selections.append(None)
                continue
            status, selected = shared.parse_selection(
                row["received"] if row["status"] == 200 else None,
                source["request"],
                source["k"],
            )
            statuses.append(status)
            selections.append(selected)
        same = all(item is not None and item == selections[0] for item in selections)
        details.append(
            {
                "id": source["id"],
                "k": source["k"],
                "source_sha256": source["source_sha256"],
                "request_sha256": source["request_sha256"],
                "parse_statuses": statuses,
                "selected_ids": selections,
                "selection_agreement": same,
                "recorded_applied_ids": source["applied_ids"],
            }
        )
    same_count = sum(item["selection_agreement"] for item in details)
    parse_ok = sum(
        status == "ok" for item in details for status in item["parse_statuses"]
    )
    return {
        "phase": "development_smoke",
        "source_fixtures": len(sources),
        "replays_expected": len(sources) * REPEATS,
        "replays_started": len(starts),
        "replays_finished": len(previous),
        "parse_ok": {"k": parse_ok, "n": len(sources) * REPEATS},
        "same_selection": {
            "k": same_count,
            "n": len(sources),
            "wilson95": shared.wilson(same_count, len(sources)),
        },
        "threshold": 0.90,
        "passes_90_percent_lower_bound": shared.wilson(same_count, len(sources))[0]
        >= 0.90,
        "details": details,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("collect", "analyze", "verify"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--key-file", type=Path)
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    try:
        sources = load_sources(args.source)
        if args.command == "collect":
            if args.key_file is None:
                raise ValueError("key file required for collect")
            collect(sources, args.key_file, args.raw)
            return 0
        summary = analyze(sources, args.raw)
        rendered = (
            json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
        )
        if args.command == "verify":
            if (
                summary["replays_started"] != summary["replays_expected"]
                or args.summary.read_text(encoding="utf-8") != rendered
            ):
                raise ValueError("replay completeness or summary byte check failed")
        else:
            args.summary.parent.mkdir(parents=True, exist_ok=True)
            args.summary.write_text(rendered, encoding="utf-8")
        print(
            json.dumps(
                {key: value for key, value in summary.items() if key != "details"},
                ensure_ascii=False,
            )
        )
    except (ValueError, OSError, KeyError) as failure:
        print(str(failure), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
