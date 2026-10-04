"""라벨을 읽지 않는 Jev 요청과 예산 제한 수집을 수행한다."""

from __future__ import annotations

import concurrent.futures
import http.client
import json
import math
import os
import random
import time
import urllib.error
import urllib.request
from typing import Any
from urllib.error import HTTPError

from storage import (
    BANDS,
    PRIVATE,
    SEED,
    append_row,
    now,
    previous,
    read_json,
    read_rows,
    reserve_call,
)

ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-1.13.0"
BASE_QUESTION = "The user's latest input sets a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do."
ALT_QUESTION = "Does the latest user input explicitly establish an ongoing rule for how future work is performed, rather than request a task, describe a product requirement, or give a one-time instruction? A narrow file or project scope alone does not make an ongoing rule one-time."
REPLACE_QUESTION = (
    "Following the later constraint makes it impossible to keep the earlier constraint."
)
RELEASE_QUESTION = (
    "The later input says the earlier constraint no longer needs to be followed."
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(
        self, req: Any, fp: Any, code: int, msg: str, headers: Any, newurl: str
    ) -> None:
        raise HTTPError(req.full_url, code, "redirect denied", headers, fp)


def parse_answers(reply: dict, questions: dict) -> dict:
    if not isinstance(reply, dict):
        return {name: None for name in questions}
    answers = reply.get("answers", {})
    if not isinstance(answers, dict) or set(answers) != set(questions):
        return {name: None for name in questions}
    parsed = {}
    for name, value in answers.items():
        probability = value.get("noul") if isinstance(value, dict) else None
        valid = type(probability) in (int, float) and math.isfinite(probability)
        parsed[name] = probability if valid and 0 <= probability <= 1 else None
    return parsed


def load_receipts() -> dict[str, dict]:
    return {row["trial_id"]: row for row in read_rows(PRIVATE / "jev.jsonl")}


# cost: io 1 HTTPS request; basis: estimate
def post_trial(trial: dict, repeat: int) -> dict:
    trial_id = f"{trial['trial_id']}-r{repeat}"
    reserve_call("jev", trial_id)
    key = os.environ.get("SATURN_JUDGE_KEY")
    if not key:
        raise RuntimeError("SATURN_JUDGE_KEY missing")
    body = {name: trial[name] for name in ("model", "state", "questions")}
    encoded = json.dumps(body, ensure_ascii=False).replace(key, "[secret]").encode()
    body = json.loads(encoded)
    request = urllib.request.Request(
        ENDPOINT,
        data=encoded,
        method="POST",
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    start = time.monotonic()
    row = {
        "trial_id": trial_id,
        "repeat": repeat,
        "meta": trial["meta"],
        "request": body,
        "ts_utc": now(),
        "http_status": None,
    }
    try:
        with urllib.request.build_opener(NoRedirect).open(
            request, timeout=30
        ) as response:
            text = response.read().decode().replace(key, "[secret]")
            reply = json.loads(text)
            row.update(
                http_status=response.status,
                raw_response=reply,
                probabilities=parse_answers(reply, body["questions"]),
                model=reply.get("model") if isinstance(reply, dict) else None,
            )
            row["status"] = (
                "ok"
                if all(p is not None for p in row["probabilities"].values())
                else "invalid"
            )
    except urllib.error.HTTPError as error:
        text = error.read().decode(errors="replace").replace(key, "[secret]")
        row.update(
            status="http_error",
            http_status=error.code,
            error_response=previous.mask_text(text),
        )
        lowered = text.lower()
        row["model_unavailable"] = "model" in lowered and any(
            marker in lowered
            for marker in (
                "not found",
                "unsupported",
                "not available",
                "does not exist",
                "not supported",
            )
        )
    except (OSError, http.client.HTTPException, ValueError) as error:
        row.update(status="failed", error=type(error).__name__)
    row["latency_ms"] = (time.monotonic() - start) * 1000
    return row


def run_repeats(trial: dict, receipts: dict[str, dict]) -> list[dict]:
    rows = []
    pending = []
    reserved = {row["trial_id"] for row in read_rows(PRIVATE / "calls.jsonl")}
    for repeat in (1, 2, 3):
        trial_id = f"{trial['trial_id']}-r{repeat}"
        if trial_id in receipts:
            rows.append(receipts[trial_id])
        elif trial_id not in reserved:
            pending.append(repeat)
        else:
            incomplete = {
                "trial_id": trial_id,
                "repeat": repeat,
                "status": "incomplete",
                "meta": trial["meta"],
                "request": {k: trial[k] for k in ("model", "state", "questions")},
            }
            append_row(PRIVATE / "jev.jsonl", incomplete)
            receipts[trial_id] = incomplete
            rows.append(incomplete)
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        for row in executor.map(lambda repeat: post_trial(trial, repeat), pending):
            append_row(PRIVATE / "jev.jsonl", row)
            receipts[row["trial_id"]] = row
            rows.append(row)
    if any(
        row.get("http_status") in (401, 403) or row.get("model_unavailable")
        for row in rows
    ):
        raise RuntimeError("judge authentication or requested model rejected")
    return sorted(rows, key=lambda row: row["repeat"])


def question(text: str) -> dict:
    return {"type": "noul", "instructions": text}


def make_route(conversation: dict, turn: dict, candidates: list[dict]) -> dict:
    index = turn["turn_index"]
    state = {
        "previous_user_input": conversation["turns"][index - 1]["text"]
        if index
        else "",
        "latest_user_input": turn["text"],
        "prior_constraint_ids": [row["turn_id"] for row in candidates],
        "active_constraint_count": len(candidates),
    }
    return {
        "trial_id": f"base-{conversation['conversation_id']}-{turn['turn_id']}",
        "model": MODEL,
        "state": state,
        "questions": {"is_constraint": question(BASE_QUESTION)},
        "meta": {
            "kind": "base",
            "conversation_id": conversation["conversation_id"],
            "turn_id": turn["turn_id"],
            "length_band": conversation["length_band"],
            "version": "route@1.1",
        },
    }


# cost: io 3n HTTPS calls; vars: n = distinct turns; basis: estimate
def collect_routes(conversations: list[dict]) -> None:
    receipts = load_receipts()
    for conversation in conversations:
        candidates = []
        for turn in conversation["turns"]:
            trial = make_route(conversation, turn, candidates)
            row = run_repeats(trial, receipts)[0]
            probability = row.get("probabilities", {}).get("is_constraint")
            if probability is not None and probability >= 0.5:
                candidates.append(turn)
            if (turn["turn_index"] + 1) % 40 == 0:
                print(
                    json.dumps(
                        {
                            "phase": "route",
                            "project": conversation["conversation_id"],
                            "turn": turn["turn_index"] + 1,
                            "receipts": len(receipts),
                        }
                    ),
                    flush=True,
                )


def select_alternatives(conversations: list[dict]) -> list[tuple[dict, dict]]:
    randomizer = random.Random(SEED)
    selected = []
    for band in BANDS:
        members = [
            (conversation, turn)
            for conversation in conversations
            if conversation["length_band"] == band
            for turn in conversation["turns"]
        ]
        randomizer.shuffle(members)
        selected.extend(members[:100])
    return selected


def collect_alternatives(conversations: list[dict]) -> None:
    receipts = load_receipts()
    for conversation, turn in select_alternatives(conversations):
        base_id = f"base-{conversation['conversation_id']}-{turn['turn_id']}"
        base = receipts.get(base_id + "-r1")
        if not base:
            continue
        trial = {
            "trial_id": base_id.replace("base-", "alternative-", 1),
            **base["request"],
            "meta": {**base["meta"], "kind": "alternative", "version": "exploratory"},
        }
        trial["questions"] = {"is_constraint": question(ALT_QUESTION)}
        run_repeats(trial, receipts)


def make_relation(conversation: dict, turn: dict, candidates: list[dict]) -> dict:
    chosen = previous.top_candidates(turn, candidates)
    state = {"latest_user_input": turn["text"], "pairs": []}
    questions = {}
    for index, candidate in enumerate(chosen):
        state["pairs"].append(
            {
                "pair_index": index,
                "earlier_constraint": candidate["text"],
                "later_constraint": turn["text"],
                "turn_distance": turn["turn_index"] - candidate["turn_index"],
            }
        )
        questions[f"replaces_{index}"] = question(
            f"For pair {index}: " + REPLACE_QUESTION
        )
        questions[f"releases_{index}"] = question(
            f"For pair {index}: " + RELEASE_QUESTION
        )
    return {
        "trial_id": f"relation-{conversation['conversation_id']}-{turn['turn_id']}",
        "model": MODEL,
        "state": state,
        "questions": questions,
        "meta": {
            "kind": "relation",
            "conversation_id": conversation["conversation_id"],
            "turn_id": turn["turn_id"],
            "length_band": conversation["length_band"],
            "candidate_ids": [row["turn_id"] for row in chosen],
            "version": "constraint@1.0",
            "release_version": "constraint@1.1-auxiliary",
        },
    }


def select_relations(conversations: list[dict], receipts: dict) -> list[dict]:
    by_band = {band: [] for band in BANDS}
    randomizer = random.Random(SEED)
    for conversation in conversations:
        candidates = []
        for turn in conversation["turns"]:
            if candidates:
                by_band[conversation["length_band"]].append(
                    make_relation(conversation, turn, candidates)
                )
            key = f"base-{conversation['conversation_id']}-{turn['turn_id']}-r1"
            probability = (
                receipts.get(key, {}).get("probabilities", {}).get("is_constraint")
            )
            if probability is not None and probability >= 0.5:
                candidates.append(turn)
    for trials in by_band.values():
        randomizer.shuffle(trials)
    selected = []
    n = sum(len(conversation["turns"]) for conversation in conversations)
    capacity = (8000 - 3 * n - 3 * len(select_alternatives(conversations))) // 3
    while len(selected) < capacity and any(by_band.values()):
        for band in BANDS:
            if by_band[band] and len(selected) < capacity:
                selected.append(by_band[band].pop())
    return selected


# cost: io 3n HTTPS calls; vars: n = selected relationship requests; basis: estimate
def collect_relations(conversations: list[dict]) -> None:
    receipts = load_receipts()
    trials = select_relations(conversations, receipts)
    for index, trial in enumerate(trials):
        run_repeats(trial, receipts)
        if (index + 1) % 40 == 0:
            print(
                json.dumps(
                    {"phase": "relation", "completed": index + 1, "total": len(trials)}
                ),
                flush=True,
            )


def load_conversations() -> list[dict]:
    selection = read_json(PRIVATE / "selection.json")
    return [
        read_json(PRIVATE / "conversations" / (row["conversation_id"] + ".json"))
        for row in sorted(
            selection["selected"],
            key=lambda row: (row["user_turns"], row["conversation_id"]),
        )
    ]
