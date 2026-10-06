#!/usr/bin/env python3
"""기존 개발 표본에 두 개의 독립된 Jev 관계 질문을 한 번씩 보낸다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[2]
RUNTIME = ROOT / ".runtime" / "continuation-atomic-judgment"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-1.13.0"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req: object, fp: object, code: int, msg: str, headers: object, newurl: str) -> None:
        raise urllib.error.HTTPError(req.full_url, code, "redirect denied", headers, fp)


def request_for(case: dict) -> dict:
    activity = "running" if case["running"] else "idle"
    context = dict(input=case["previous_input"], goal_excerpt=case["goal_excerpt"], progress_excerpt=case["progress_excerpt"])
    state = f"chat: {activity}\nprevious input handled as: queue\nuser input: {case['input']}\nprevious task context: " + json.dumps(context, ensure_ascii=False, separators=(",", ":"))
    return dict(model=MODEL, state=state, questions={
        "same_goal": dict(type="noul", instructions="Does the new user input continue, correct, complete, or verify the previous task described in the context?"),
        "independent_goal": dict(type="noul", instructions="Does the new user input start a separate goal that can be handled without the previous task's result?"),
    })


def save(path: Path, value: dict, secret: str) -> None:
    body = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).replace(secret, "[secret]") + "\n"
    with path.open("x", encoding="utf-8") as stream:
        stream.write(body)
        stream.flush()
        os.fsync(stream.fileno())


def call(case: dict, run: Path, secret: str) -> str:
    trial = case["id"]
    body = request_for(case)
    encoded = json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode()
    save(run / f"{trial}.reservation.json", dict(trial_id=trial, ts_utc=datetime.now(timezone.utc).isoformat(), request_sha256=hashlib.sha256(encoded).hexdigest()), secret)
    request = urllib.request.Request(ENDPOINT, data=encoded, method="POST", headers={"Authorization": "Bearer " + secret, "Content-Type": "application/json"})
    result = dict(trial_id=trial, request=body, request_bytes=len(encoded), status="unknown", http_status=None, response=None)
    started = time.monotonic()
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=45) as response:
            result.update(http_status=response.status, response=json.loads(response.read().decode("utf-8")), status="ok")
    except urllib.error.HTTPError as error:
        result.update(http_status=error.code, response=error.read().decode("utf-8", errors="replace"), status="http_error")
    except (OSError, ValueError, TypeError) as error:
        result.update(status="failed", error=type(error).__name__)
    result["elapsed_ms"] = round((time.monotonic() - started) * 1000, 3)
    save(run / f"{trial}.json", result, secret)
    return result["status"]


def main() -> None:
    source = Path(os.environ["SATURN_CONTINUATION_SAMPLE"])
    secret = Path(os.environ["SATURN_KEY_FILE"]).read_text(encoding="utf-8").strip()
    if not source.is_file() or not secret:
        raise RuntimeError("missing source or router key")
    os.umask(0o077)
    cases = json.loads(source.read_text(encoding="utf-8"))
    if len(cases) != 608 or len({case["id"] for case in cases}) != 608:
        raise RuntimeError("sample size or ids changed")
    commit = subprocess.check_output(["git", "rev-parse", "--short=7", "HEAD"], cwd=ROOT, text=True).strip()
    run_id = os.environ.get("SATURN_RUN_ID") or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + commit
    run = RUNTIME / run_id
    run.mkdir(parents=True, exist_ok=False)
    save(run / "manifest.json", dict(run_id=run_id, sample_sha256=hashlib.sha256(source.read_bytes()).hexdigest(), n=len(cases), model=MODEL, source_kind="existing development sample"), secret)
    failed = 0
    ordered = sorted(cases, key=lambda item: item["id"])
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        for start in range(0, len(ordered), 8):
            futures = [executor.submit(call, case, run, secret) for case in ordered[start : start + 8]]
            for index, future in enumerate(futures, start + 1):
                status = future.result()
                failed = failed + 1 if status != "ok" else 0
                if index % 50 == 0:
                    print(f"{index}/{len(cases)} collected", flush=True)
                if failed >= 10:
                    raise RuntimeError("ten consecutive request failures")
    print(run_id, flush=True)


if __name__ == "__main__":
    main()
