"""합의 라벨을 state로만 사용해 Jev 조건 A·B를 반복 측정한다."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import random
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (  # noqa: E402
    PRIVATE_ROOT,
    ensure_private_root,
    mask_text,
    read_jsonl,
    top_candidates,
    write_json,
)

ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-1.13.0"
SEED = 316
REPEATS = 3
MAX_CALLS = 1500
REPLACE_THRESHOLD = 0.8
POSSIBLE_THRESHOLD = 0.5


def load_consensus() -> dict[str, dict[str, dict]]:
    by_labeler: dict[str, dict[str, dict]] = defaultdict(dict)
    for path in sorted((PRIVATE_ROOT / "labels").glob("*.jsonl")):
        for row in read_jsonl(path):
            if row.get("status") == "ok" and isinstance(row.get("parsed"), dict):
                by_labeler[row["labeler"]][row["conversation_id"]] = row["parsed"]
    labelers = sorted(by_labeler)
    if len(labelers) < 2:
        raise RuntimeError("두 라벨러의 응답이 모두 필요하다")
    consensus: dict[str, dict[str, dict]] = {}
    for conversation_id in set(by_labeler[labelers[0]]) & set(by_labeler[labelers[1]]):
        left = {row["turn_id"]: row for row in by_labeler[labelers[0]][conversation_id].get("turns", [])}
        right = {row["turn_id"]: row for row in by_labeler[labelers[1]][conversation_id].get("turns", [])}
        turns = {}
        for turn_id in set(left) & set(right):
            a, b = left[turn_id], right[turn_id]
            if (
                a.get("is_constraint") == b.get("is_constraint")
                and a.get("operation", "none") == b.get("operation", "none")
                and sorted(a.get("targets", [])) == sorted(b.get("targets", []))
            ):
                turns[turn_id] = {
                    "is_constraint": bool(a.get("is_constraint")),
                    "operation": a.get("operation", "none"),
                    "targets": sorted(a.get("targets", [])),
                }
        final_left = sorted(by_labeler[labelers[0]][conversation_id].get("final_active_turn_ids", []))
        final_right = sorted(by_labeler[labelers[1]][conversation_id].get("final_active_turn_ids", []))
        consensus[conversation_id] = {
            "turns": turns,
            "final_set": set(final_left) if final_left == final_right else set(),
            "final_agree": final_left == final_right,
        }
    return consensus


def active_before(rows: list[dict], labels: dict[str, dict]) -> dict[str, list[dict]]:
    active: list[dict] = []
    before: dict[str, list[dict]] = {}
    for row in rows:
        before[row["turn_id"]] = list(active)
        label = labels.get(row["turn_id"])
        if not label or not label["is_constraint"]:
            continue
        targets = set(label["targets"])
        if label["operation"] in ("replace", "release"):
            active = [candidate for candidate in active if candidate["turn_id"] not in targets]
        if label["operation"] != "release":
            active.append(row)
    return before


def question(question_id: str, instructions: str) -> dict:
    return {question_id: {"type": "noul", "instructions": instructions}}


def masked_constraint(row: dict) -> dict:
    return {
        "turn_id": row["turn_id"],
        "turn_index": row["turn_index"],
        "constraint": mask_text(row["text"]),
        "files": [mask_text(path) for path in row.get("files", [])],
    }


def make_trials(rows: list[dict], consensus: dict[str, dict[str, dict]]) -> list[dict]:
    randomizer = random.Random(SEED)
    by_conversation: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        by_conversation[row["conversation_id"]].append(row)
    trials = []
    for conversation_id, conversation_rows in sorted(by_conversation.items()):
        conversation_rows.sort(key=lambda row: row["turn_index"])
        consensus_row = consensus.get(conversation_id)
        if not consensus_row:
            continue
        labels = consensus_row["turns"]
        before_map = active_before(conversation_rows, labels)
        sampled_rows = [row for row in conversation_rows if row.get("sampled")]
        final_active = [row for row in conversation_rows if row["turn_id"] in consensus_row["final_set"]]
        for row in sampled_rows:
            previous = conversation_rows[row["turn_index"] - 1]["text"] if row["turn_index"] else ""
            active = before_map[row["turn_id"]]
            for condition in ("A_top10_overlap", "B_all_active"):
                candidates = top_candidates(row, active) if condition == "A_top10_overlap" else list(active)
                questions = question(
                    "is_constraint",
                    "The user's latest input sets a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do.",
                )
                for index, candidate in enumerate(candidates, start=1):
                    questions.update(question(
                        f"replaces_{index}",
                        "Following the later constraint makes it impossible to keep the earlier constraint.",
                    ))
                state = {
                    "conversation_id": conversation_id,
                    "turn_id": row["turn_id"],
                    "previous_user_input": mask_text(previous),
                    "latest_user_input": mask_text(row["text"]),
                    "prior_constraint_ids": [candidate["turn_id"] for candidate in candidates],
                    "active_constraint_count": len(active),
                    "existing_constraints": [masked_constraint(candidate) for candidate in candidates],
                }
                base = {
                    "model": MODEL,
                    "state": state,
                    "questions": questions,
                    "meta": {
                        "conversation_id": conversation_id,
                        "turn_id": row["turn_id"],
                        "length_band": row["length_band"],
                        "condition": condition,
                        "candidate_ids": [candidate["turn_id"] for candidate in candidates],
                    },
                }
                for repeat in range(1, REPEATS + 1):
                    trial = json.loads(json.dumps(base, ensure_ascii=False))
                    trial["meta"]["repeat"] = repeat
                    trials.append(trial)
        if len(final_active) >= 11:
            latest = conversation_rows[-1]
            for condition in ("A_top10_overlap", "B_all_active"):
                candidates = top_candidates(latest, final_active) if condition == "A_top10_overlap" else list(final_active)
                questions = {}
                for index, candidate in enumerate(candidates, start=1):
                    questions.update(question(
                        f"keep_{index}",
                        "This constraint should remain active for the user's work at the end of this conversation.",
                    ))
                base = {
                    "model": MODEL,
                    "state": {
                        "conversation_id": conversation_id,
                        "latest_user_input": mask_text(latest["text"]),
                        "active_constraint_count": len(final_active),
                        "candidate_constraints": [masked_constraint(candidate) for candidate in candidates],
                    },
                    "questions": questions,
                    "meta": {
                        "conversation_id": conversation_id,
                        "turn_id": latest["turn_id"],
                        "length_band": latest["length_band"],
                        "condition": condition,
                        "kind": "keep",
                        "candidate_ids": [candidate["turn_id"] for candidate in candidates],
                        "gold_final_ids": sorted(consensus_row["final_set"]),
                    },
                }
                for repeat in range(1, REPEATS + 1):
                    trial = json.loads(json.dumps(base, ensure_ascii=False))
                    trial["meta"]["repeat"] = repeat
                    trials.append(trial)
    randomizer.shuffle(trials)
    for index, trial in enumerate(trials, start=1):
        trial["meta"]["trial_id"] = f"t-{index:04d}"
    return trials


def key_from_environment() -> str | None:
    key = os.environ.get("SATURN_JUDGE_KEY")
    if key:
        return key
    result = subprocess.run(
        ["security", "find-generic-password", "-a", "saturn-key", "-w"],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 and result.stdout.strip() else None


def post(body: dict, key: str) -> tuple[int | None, dict | None, str | None]:
    encoded = json.dumps({k: v for k, v in body.items() if k != "meta"}, ensure_ascii=False).encode("utf-8")
    send_failures = 0
    rate_limits = 0
    while True:
        request = urllib.request.Request(
            ENDPOINT,
            data=encoded,
            method="POST",
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.status, json.loads(response.read().decode("utf-8")), None
        except urllib.error.HTTPError as error:
            if error.code in (429, 529) and rate_limits < 3:
                rate_limits += 1
                time.sleep(float(error.headers.get("retry-after") or 2.0))
                continue
            return error.code, None, f"http {error.code}"
        except urllib.error.URLError as error:
            if isinstance(error.reason, (ConnectionRefusedError, socket.gaierror)) and send_failures < 3:
                send_failures += 1
                continue
            return None, None, type(error.reason).__name__
        except TimeoutError:
            return None, None, "timeout"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    ensure_private_root()
    selected_path = PRIVATE_ROOT / "selected.jsonl"
    if not selected_path.exists():
        if args.dry_run:
            write_json(PRIVATE_ROOT / "jev-dry-run.json", {
                "seed": SEED,
                "trial_count": 0,
                "request_keys": ["model", "state", "questions"],
                "note": "selected.jsonl 없음 상태의 형식 드라이런",
            })
            print("Jev 드라이런 통과: selected.jsonl 없이 요청 형식만 확인")
            return 0
        raise RuntimeError("selected.jsonl이 없다")
    rows = read_jsonl(selected_path)
    trials = make_trials(rows, load_consensus())
    if len(trials) > MAX_CALLS:
        raise RuntimeError(f"Jev 호출 예정 수 {len(trials)}가 상한 1,500회를 넘는다")
    dry_run = {
        "created_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "seed": SEED,
        "trial_count": len(trials),
        "conditions": {condition: sum(trial["meta"]["condition"] == condition for trial in trials) for condition in ("A_top10_overlap", "B_all_active")},
        "max_calls": MAX_CALLS,
        "request_keys": sorted({key for trial in trials for key in trial if key != "meta"}),
    }
    write_json(PRIVATE_ROOT / "jev-dry-run.json", dry_run)
    if args.dry_run:
        print(f"Jev 드라이런 통과: 요청 {len(trials)}회, 상한 {MAX_CALLS}회")
        return 0
    key = key_from_environment()
    if not key:
        print("SATURN_JUDGE_KEY 없음: 키 없이 요청 형식만 확인하고 Jev 수집을 멈춘다", file=sys.stderr)
        return 2
    output_path = PRIVATE_ROOT / "jev" / "responses.jsonl"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    existing = len(read_jsonl(output_path)) if output_path.exists() else 0
    if existing + len(trials) > MAX_CALLS:
        raise RuntimeError("기존 Jev 호출과 이번 호출의 합이 상한 1,500회를 넘는다")
    with output_path.open("a", encoding="utf-8", newline="\n") as output:
        for trial in trials:
            sent = time.monotonic()
            status_code, reply, error = post(trial, key)
            latency_ms = round((time.monotonic() - sent) * 1000)
            answers = reply.get("answers") if isinstance(reply, dict) else None
            output.write(json.dumps({
                "trial_id": trial["meta"]["trial_id"],
                "meta": trial["meta"],
                "status": "ok" if isinstance(answers, dict) else "invalid",
                "http_status": status_code,
                "answers": answers,
                "model": reply.get("model") if isinstance(reply, dict) else None,
                "latency_ms": latency_ms,
                "error": error,
                "request": {key: value for key, value in trial.items() if key != "meta"},
            }, ensure_ascii=False) + "\n")
            output.flush()
    print(f"Jev 수집 끝: 요청 {len(trials)}회")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
