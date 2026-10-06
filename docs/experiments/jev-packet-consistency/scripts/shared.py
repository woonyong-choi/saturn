"""Common Jev replay, ledger, selection, and interval rules."""

from __future__ import annotations

import hashlib
import json
import math
import os
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, TextIO

ENDPOINT = "https://api.typesafe.ai/v1/systemone"
Z = 1.959963984540054


def request_sha256(request_text: str) -> str:
    return hashlib.sha256(request_text.encode("utf-8")).hexdigest()


def append_event(stream: TextIO, record: dict[str, Any]) -> None:
    stream.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
    stream.flush()
    os.fsync(stream.fileno())


def ledger(
    path: Path, key_fields: tuple[str, ...]
) -> tuple[
    dict[tuple[Any, ...], dict[str, Any]], dict[tuple[Any, ...], dict[str, Any]]
]:
    starts: dict[tuple[Any, ...], dict[str, Any]] = {}
    ends: dict[tuple[Any, ...], dict[str, Any]] = {}
    if not path.exists():
        return starts, ends
    for line in path.read_text(encoding="utf-8").splitlines():
        record = json.loads(line)
        key = tuple(record[name] for name in key_fields)
        event = record["event"]
        if event == "start":
            if key in starts:
                raise ValueError(f"duplicate trial start: {key}")
            starts[key] = record
        elif event == "end":
            if key in ends:
                raise ValueError(f"duplicate trial end: {key}")
            ends[key] = record
        else:
            raise ValueError("unknown ledger event")
    if not set(ends) <= set(starts):
        raise ValueError("end without start")
    return starts, ends


def send(request_text: str, key: str) -> dict[str, Any]:
    request = urllib.request.Request(
        ENDPOINT,
        data=request_text.encode("utf-8"),
        method="POST",
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    status: int | None = None
    received = ""
    error = ""
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            status = response.status
            received = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as failure:
        status = failure.code
        received = failure.read().decode("utf-8", errors="replace")
        error = f"http {status}"
    except (urllib.error.URLError, TimeoutError) as failure:
        error = type(failure).__name__
    return {
        "status": status,
        "error": error,
        "elapsed_ms": round((time.monotonic() - started) * 1000),
        "received": received.replace(key, "[redacted]"),
    }


def collect_trials(
    trials: list[dict[str, Any]],
    key_fields: tuple[str, ...],
    key_file: Path,
    output: Path,
    *,
    call_cap: int | None = None,
) -> None:
    key = key_file.read_text(encoding="utf-8").strip()
    if not key:
        raise ValueError("empty key file")
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    output.parent.chmod(0o700)
    output.touch(mode=0o600, exist_ok=True)
    output.chmod(0o600)
    starts, _ = ledger(output, key_fields)
    with output.open("a", encoding="utf-8") as stream:
        for trial in trials:
            trial_key = tuple(trial[name] for name in key_fields)
            if trial_key in starts:
                continue
            if call_cap is not None and len(starts) >= call_cap:
                raise ValueError("call cap reached")
            base = {name: trial[name] for name in key_fields}
            base.update(trial.get("metadata", {}))
            base["request_sha256"] = request_sha256(trial["request_text"])
            append_event(stream, {"event": "start", **base})
            starts[trial_key] = base
            result = send(trial["request_text"], key)
            append_event(stream, {"event": "end", **base, **result})
            if result["status"] in (401, 403):
                raise ValueError("authentication rejected")


def parse_selection(
    received: str | None, request_text: str, k: int
) -> tuple[str, list[int] | None]:
    if not received:
        return "missing", None
    try:
        answers = json.loads(received)["answers"]
        questions = json.loads(request_text)["questions"]
        if not isinstance(answers, dict) or set(answers) != set(questions):
            return "question_mismatch", None
        scores: dict[int, float] = {}
        for name, answer in answers.items():
            parts = name.split("_")
            if (
                len(parts) != 3
                or parts[0] not in ("call", "result")
                or parts[2] != "keep"
            ):
                return "question_id", None
            item_id = int(parts[1])
            probability = answer["noul"]
            if (
                type(probability) not in (int, float)
                or not math.isfinite(probability)
                or not 0 <= probability <= 1
            ):
                return "probability", None
            scores[item_id] = max(scores.get(item_id, 0), float(probability))
        if k > len(scores):
            return "k_exceeds_candidates", None
        return "ok", sorted(
            sorted(scores, key=lambda item_id: (-scores[item_id], item_id))[:k]
        )
    except (KeyError, TypeError, ValueError, json.JSONDecodeError):
        return "invalid_json", None


def wilson(success: int, total: int) -> list[float] | None:
    if total == 0:
        return None
    proportion = success / total
    denominator = 1 + Z * Z / total
    center = (proportion + Z * Z / (2 * total)) / denominator
    radius = (
        Z
        * math.sqrt(proportion * (1 - proportion) / total + Z * Z / (4 * total * total))
        / denominator
    )
    return [round(center - radius, 6), round(center + radius, 6)]
