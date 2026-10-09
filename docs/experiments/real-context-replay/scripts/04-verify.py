"""보관된 원자료와 분석 결과의 hash 및 호출 완전성을 검사한다."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]


def verify_responses() -> int:
    root = ROOT / ".local/experiments/real-context-replay"
    count = 0
    for phase in [root, root / "corrected"]:
        for path in (phase / "raw").glob("*/result.json"):
            result = json.loads(path.read_text())
            if result["status"] != "ok":
                continue
            usages, answer = raw_response(path.parent, result["provider"])
            if usages != result["usages"] or answer != result["text"]:
                raise RuntimeError(f"parsed response differs from raw data: {path}")
            count += 1
    return count


def raw_response(folder: Path, provider: str) -> tuple[list[dict], str]:
    if provider == "claude":
        events = []
        for name in ["ready.jsonl", "answer.jsonl"]:
            rows = [
                json.loads(line) for line in (folder / name).read_text().splitlines()
            ]
            events.append(
                next(row for row in reversed(rows) if row.get("type") == "result")
            )
        return [event.get("usage", {}) for event in events], events[-1].get(
            "result", ""
        )
    events = [
        json.loads(line) for line in (folder / "stdout.jsonl").read_text().splitlines()
    ]
    usages = [
        event["usage"] for event in events if event.get("type") == "turn.completed"
    ]
    answers = [
        event["item"]["text"]
        for event in events
        if event.get("type") == "item.completed"
        and event.get("item", {}).get("type") == "agent_message"
    ]
    return usages, answers[-1]


def main() -> None:
    count = 0
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        expected, relative = line.split("  ", 1)
        path = ROOT / relative
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            raise RuntimeError(f"evidence hash mismatch: {relative}")
        count += 1
    summary = json.loads((PUBLIC / "results/summary.json").read_text())
    if summary["planned_sessions"] != summary["collected_sessions"]:
        raise RuntimeError("planned sessions are missing")
    for trial in summary["trials"]:
        if trial["questions"] != len(trial["checks"]):
            raise RuntimeError("question denominator mismatch")
    print(
        json.dumps(
            {
                "verified_files": count,
                "sessions": summary["collected_sessions"],
                "raw_responses_reconciled": verify_responses(),
            }
        )
    )


if __name__ == "__main__":
    main()
