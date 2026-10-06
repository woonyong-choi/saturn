"""Collect and check repeated Jev packet judgments from sealed task fixtures."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import sys
from pathlib import Path
from typing import Any

import shared

ROOT = Path(__file__).resolve().parent.parent
VARIANT_REPEATS = {"identical": 5, "order": 3, "neutral": 3, "critical": 3}


def canonical(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def fixtures(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        raise ValueError(f"fixture file missing: {path}")
    rows = [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not rows:
        raise ValueError("fixture file is empty")
    seen: set[str] = set()
    for row in rows:
        fixture_id = row["task_id"]
        if fixture_id in seen:
            raise ValueError(f"duplicate fixture: {fixture_id}")
        seen.add(fixture_id)
        if row["phase"] not in {"dev", "confirm"}:
            raise ValueError(f"invalid phase: {fixture_id}")
        if not all(
            row.get(name)
            for name in (
                "project_family",
                "source_sha256",
                "candidate_sha256",
                "goal_sha256",
                "policy_sha256",
                "compact_request_sha256",
            )
        ):
            raise ValueError(f"missing fingerprint: {fixture_id}")
        if type(row["k"]) is not int or row["k"] < 1:
            raise ValueError(f"invalid k: {fixture_id}")
        variants = row["requests"]
        if set(variants) != set(VARIANT_REPEATS):
            raise ValueError(f"invalid variants: {fixture_id}")
        base = variants["identical"]
        names = set(base["questions"])
        if (
            set(variants["order"]["questions"]) != names
            or variants["order"]["state"] != base["state"]
            or variants["order"]["questions"] != base["questions"]
        ):
            raise ValueError(f"order variant changes content: {fixture_id}")
        if list(variants["order"]["questions"]) == list(base["questions"]):
            raise ValueError(f"order variant keeps question order: {fixture_id}")
        if (
            variants["neutral"]["state"] == base["state"]
            or variants["critical"]["state"] == base["state"]
        ):
            raise ValueError(f"state variant is unchanged: {fixture_id}")
        if json.loads(row["raw_request"]) != base:
            raise ValueError(f"raw request differs from fixture: {fixture_id}")
        if (
            hashlib.sha256(row["raw_request"].encode("utf-8")).hexdigest()
            != row["compact_request_sha256"]
        ):
            raise ValueError(f"compact request hash mismatch: {fixture_id}")
        if row["k"] > len({int(name.split("_")[1]) for name in names}):
            raise ValueError(f"k exceeds candidates: {fixture_id}")
        for variant, request in variants.items():
            if (
                request.get("model") != base.get("model")
                or set(request["questions"]) != names
            ):
                raise ValueError(f"candidate set changed: {fixture_id}/{variant}")
            if not all(
                name.startswith(("call_", "result_")) and name.endswith("_keep")
                for name in names
            ):
                raise ValueError(f"invalid question id: {fixture_id}")
            if any("expected" in key or "gold" in key for key in request):
                raise ValueError(f"label in request: {fixture_id}/{variant}")
        for field in ("required_ids", "critical_required_ids"):
            if not set(row[field]) <= {int(name.split("_")[1]) for name in names}:
                raise ValueError(f"unknown evidence id: {fixture_id}/{field}")
    families: dict[str, str] = {}
    for row in rows:
        previous = families.setdefault(row["project_family"], row["phase"])
        if previous != row["phase"]:
            raise ValueError(f"project family crosses phases: {row['project_family']}")
    return rows


def trials(rows: list[dict[str, Any]], phase: str) -> list[dict[str, Any]]:
    planned = [
        {
            "id": row["task_id"],
            "phase": phase,
            "variant": variant,
            "rep": rep,
            "request": row["requests"][variant],
            "raw_request": row["raw_request"] if variant == "identical" else None,
        }
        for row in rows
        if row["phase"] == phase
        for variant, count in VARIANT_REPEATS.items()
        for rep in range(1, count + 1)
    ]
    random.Random(5447).shuffle(planned)
    return planned


def collect(
    rows: list[dict[str, Any]], phase: str, key_file: Path, output: Path
) -> None:
    if phase == "confirm":
        lock_path = ROOT / "eval" / "lock.json"
        if not lock_path.exists():
            raise ValueError("confirmation lock missing")
        lock = json.loads(lock_path.read_text(encoding="utf-8"))
        if lock.get("fixture_sha256") != digest(rows) or lock.get(
            "fixture_count"
        ) != len(rows):
            raise ValueError("confirmation fixture lock mismatch")
        if type(lock.get("call_cap")) is not int or lock["call_cap"] < len(rows) * sum(
            VARIANT_REPEATS.values()
        ):
            raise ValueError("confirmation call cap missing or too small")
    planned = []
    for trial in trials(rows, phase):
        request_text = trial["raw_request"] or json.dumps(
            trial["request"], ensure_ascii=False, separators=(",", ":")
        )
        planned.append(
            {
                "id": trial["id"],
                "phase": phase,
                "variant": trial["variant"],
                "rep": trial["rep"],
                "request_text": request_text,
            }
        )
    shared.collect_trials(
        planned,
        ("id", "variant", "rep"),
        key_file,
        output,
        call_cap=lock["call_cap"] if phase == "confirm" else None,
    )


def analyze(rows: list[dict[str, Any]], raw_path: Path) -> dict[str, Any]:
    starts, by_trial = shared.ledger(raw_path, ("id", "variant", "rep"))
    detail: list[dict[str, Any]] = []
    counts = {name: 0 for name in ("identical", "order", "neutral", "critical")}
    for row in rows:
        choices: dict[str, list[list[int] | None]] = {}
        parse_failed = 0
        for variant, count in VARIANT_REPEATS.items():
            choices[variant] = []
            for rep in range(1, count + 1):
                raw = by_trial.get((row["task_id"], variant, rep))
                answer = None
                expected_payload = (
                    row["raw_request"]
                    if variant == "identical"
                    else json.dumps(
                        row["requests"][variant],
                        ensure_ascii=False,
                        separators=(",", ":"),
                    )
                )
                expected_sha = shared.request_sha256(expected_payload)
                if (
                    raw
                    and raw["status"] == 200
                    and raw["request_sha256"] == expected_sha
                ):
                    _, answer = shared.parse_selection(
                        raw["received"], expected_payload, row["k"]
                    )
                if answer is None:
                    parse_failed += 1
                choices[variant].append(answer)
        base = choices["identical"]
        same = all(choice is not None and choice == base[0] for choice in base)
        valid = [choice for choice in base if choice is not None]
        base_majority = max(valid, key=lambda choice: base.count(choice), default=None)
        if (
            base_majority is not None
            and sum(
                base.count(list(choice)) == base.count(base_majority)
                for choice in {tuple(item) for item in valid}
            )
            > 1
        ):
            base_majority = None
        result = {
            "id": row["task_id"],
            "phase": row["phase"],
            "parse_failed": parse_failed,
            "identical": same,
            "order": base_majority is not None
            and all(choice == base_majority for choice in choices["order"]),
            "neutral": base_majority is not None
            and all(choice == base_majority for choice in choices["neutral"]),
            "critical": all(
                choice is not None and set(row["critical_required_ids"]) <= set(choice)
                for choice in choices["critical"]
            ),
            "required_evidence_base": all(
                choice is not None and set(row["required_ids"]) <= set(choice)
                for choice in base
            ),
            "action_pass": None,
        }
        for variant in counts:
            counts[variant] += int(result[variant])
        detail.append(result)
    total = len(rows)
    return {
        "fixtures": total,
        "counts": {
            name: {"k": count, "n": total, "wilson95": shared.wilson(count, total)}
            for name, count in counts.items()
        },
        "parse_failed": sum(item["parse_failed"] for item in detail),
        "trials_expected": total * sum(VARIANT_REPEATS.values()),
        "trials_started": len(starts),
        "trials_finished": len(by_trial),
        "detail": detail,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "command", choices=("collect", "process", "analyze", "verify", "all")
    )
    parser.add_argument("--phase", choices=("dev", "confirm"), default="dev")
    parser.add_argument(
        "--fixtures", type=Path, default=ROOT / "eval" / "fixtures.jsonl"
    )
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--key-file", type=Path)
    args = parser.parse_args()
    try:
        rows = fixtures(args.fixtures)
        phase_rows = [row for row in rows if row["phase"] == args.phase]
        if not phase_rows:
            raise ValueError(f"no {args.phase} fixtures")
        raw = (
            args.raw
            or ROOT.parents[2]
            / ".runtime"
            / "jev-packet-consistency"
            / f"{args.phase}.jsonl"
        )
        if args.command in ("collect", "all"):
            key_file = args.key_file or Path(os.environ["SATURN_JUDGE_KEY_FILE"])
            collect(phase_rows, args.phase, key_file, raw)
        if args.command in ("process", "analyze", "verify", "all"):
            summary = analyze(phase_rows, raw)
            destination = ROOT / "results" / f"{args.phase}-summary.json"
            rendered = (
                json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
            )
            if args.command == "verify":
                if summary["trials_started"] != summary["trials_expected"]:
                    raise ValueError("incomplete trial ledger")
                if (
                    not destination.exists()
                    or destination.read_text(encoding="utf-8") != rendered
                ):
                    raise ValueError("summary differs from raw")
            elif args.command != "process":
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(rendered, encoding="utf-8")
            print(
                json.dumps(
                    {key: value for key, value in summary.items() if key != "detail"},
                    ensure_ascii=False,
                )
            )
    except (ValueError, KeyError, OSError) as failure:
        print(str(failure), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
