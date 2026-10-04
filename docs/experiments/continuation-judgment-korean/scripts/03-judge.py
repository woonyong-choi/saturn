"""라벨을 읽지 않고 실제 질문 형식의 A/B 요청을 세 번 수집한다."""

from __future__ import annotations

import concurrent.futures
import http.client
import json
import math
import os
import time
import urllib.error
import urllib.request
from typing import Any

from storage import PRIVATE, append, now, read, reserve, rows, setup

MODEL = "jev-1.13.0"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
KEEP = "Does this input continue the work of the agent that handled the previous input?"
ACTIONABLE = (
    "Is this input clear enough to act on without exploring the codebase first?"
)
RELATION = "How does this input relate to the work that is running now?"
SEND = "Should this input be added to the running turn (steer), wait until the turn ends (queue), or start a separate agent (spawn)?"
RELATION_OPTIONS = ["refines", "continues", "independent", "conflicts", "other"]
SEND_OPTIONS = ["steer", "queue", "spawn", "other"]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(
        self, req: Any, fp: Any, code: int, msg: str, headers: Any, newurl: str
    ) -> None:
        raise urllib.error.HTTPError(req.full_url, code, "redirect denied", headers, fp)


def request_for(case: dict, condition: str) -> dict:
    activity = "running" if case["running"] else "idle"
    state = f"chat: {activity}\nprevious input handled as: queue\nuser input: {case['input']}"
    if condition == "B":
        context = {
            "input": case["previous_input"],
            "goal_excerpt": case["goal_excerpt"],
            "progress_excerpt": case["progress_excerpt"],
        }
        state += "\nprevious task context: " + json.dumps(
            context, ensure_ascii=False, separators=(",", ":")
        )
    questions = {
        "keep_current": {"type": "noul", "instructions": KEEP},
        "is_actionable": {"type": "noul", "instructions": ACTIONABLE},
    }
    if case["running"]:
        for name, text, options in [
            ("relation_to_running", RELATION, RELATION_OPTIONS),
            ("steer_or_spawn", SEND, SEND_OPTIONS),
        ]:
            questions[name] = {
                "type": "choice",
                "instructions": text,
                "criteria": dict.fromkeys(options),
            }
    return {"model": MODEL, "state": state, "questions": questions}


def parse(reply: dict, questions: dict) -> dict:
    answers = reply.get("answers", {})
    if not isinstance(answers, dict) or set(answers) != set(questions):
        raise ValueError("answer coverage mismatch")
    result = {}
    for name, question in questions.items():
        answer = answers[name]
        if not isinstance(answer, dict):
            raise ValueError("invalid answer")
        if question["type"] == "noul":
            values = [answer.get("noul")]
            result[name] = values[0]
        else:
            probabilities = answer.get("probabilities", {})
            if not isinstance(probabilities, dict) or set(probabilities) != set(
                question["criteria"]
            ):
                raise ValueError("choice coverage mismatch")
            values = list(probabilities.values())
            result[name] = probabilities
        if any(
            type(p) not in (int, float) or not math.isfinite(p) or not 0 <= p <= 1
            for p in values
        ):
            raise ValueError("invalid probability")
        if question["type"] == "choice" and abs(sum(values) - 1) > 0.01:
            raise ValueError("distribution sum mismatch")
    return result


def post(case: dict, condition: str, repeat: int) -> dict:
    trial = f"{case['id']}-{condition}-r{repeat}"
    key = os.environ["SATURN_JUDGE_KEY"]
    body = request_for(case, condition)
    encoded = (
        json.dumps(body, ensure_ascii=False, separators=(",", ":"))
        .replace(key, "[secret]")
        .encode()
    )
    row = {
        "run_id": read(PRIVATE / "run.json")["run_id"],
        "trial_id": trial,
        "id": case["id"],
        "condition": condition,
        "repeat": repeat,
        "ts_utc": now(),
        "request": json.loads(encoded),
        "request_bytes": len(encoded),
        "http_status": None,
    }
    if len(encoded) > 110000:
        return {**row, "status": "oversize"}
    reserve("jev", trial)
    request = urllib.request.Request(
        ENDPOINT,
        data=encoded,
        method="POST",
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    start = time.monotonic()
    try:
        with urllib.request.build_opener(NoRedirect).open(
            request, timeout=45
        ) as response:
            text = response.read().decode().replace(key, "[secret]")
            row.update(http_status=response.status, raw_response=text)
            reply = json.loads(text)
            row.update(
                model=reply.get("model"),
                answers=parse(reply, body["questions"]),
                status="ok",
            )
    except urllib.error.HTTPError as error:
        row.update(
            status="http_error",
            http_status=error.code,
            raw_response=error.read().decode(errors="replace").replace(key, "[secret]"),
        )
    except (
        OSError,
        http.client.HTTPException,
        ValueError,
        TypeError,
        AttributeError,
    ) as error:
        row.update(status="failed", error=type(error).__name__)
    row["latency_ms"] = (time.monotonic() - start) * 1000
    return row


def main() -> None:
    setup()
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("missing judge key")
    sample = read(PRIVATE / "sample.json")
    receipts = {r["trial_id"]: r for r in rows(PRIVATE / "jev.jsonl")}
    reserved = {
        r["trial_id"] for r in rows(PRIVATE / "calls.jsonl") if r["kind"] == "jev"
    }
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        for index, case in enumerate(sample):
            pending = []
            for repeat in (1, 2, 3):
                for condition in case["condition_order"]:
                    trial = f"{case['id']}-{condition}-r{repeat}"
                    if trial in receipts:
                        continue
                    if trial in reserved:
                        body = request_for(case, condition)
                        row = {
                            "run_id": read(PRIVATE / "run.json")["run_id"],
                            "trial_id": trial,
                            "id": case["id"],
                            "condition": condition,
                            "repeat": repeat,
                            "ts_utc": now(),
                            "status": "incomplete",
                            "request": body,
                            "request_bytes": len(
                                json.dumps(
                                    body, ensure_ascii=False, separators=(",", ":")
                                ).encode()
                            ),
                        }
                        append(PRIVATE / "jev.jsonl", row)
                    else:
                        pending.append(pool.submit(post, case, condition, repeat))
            stop = False
            for future in concurrent.futures.as_completed(pending):
                row = future.result()
                append(PRIVATE / "jev.jsonl", row)
                text = row.get("raw_response", "").lower()
                stop |= row.get("http_status") in (401, 403) or (
                    "model" in text
                    and any(
                        s in text
                        for s in (
                            "not found",
                            "unsupported",
                            "not available",
                            "not supported",
                        )
                    )
                )
            if stop:
                raise RuntimeError("judge authentication or model rejected")
            if (index + 1) % 10 == 0:
                print(
                    json.dumps({"completed_cases": index + 1, "total": len(sample)}),
                    flush=True,
                )


if __name__ == "__main__":
    main()
