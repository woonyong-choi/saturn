"""봉인된 표본을 한 번씩 호출하고 키 없는 요청과 응답을 보존한다."""

from __future__ import annotations

import concurrent.futures
import getpass
import hashlib
import json
import math
import os
import platform
import re
import subprocess
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/constraint-registration-generalization"
MODEL = "jev-1.13.0"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args: Any, **_kwargs: Any) -> None:
        return None


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def save_new(path: Path, value: Any) -> None:
    with path.open("x") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def hashes() -> dict:
    paths = [PUBLIC / "design.md", PUBLIC / "questions.json", PRIVATE / "samples.json"]
    paths.extend(sorted((PUBLIC / "scripts").glob("*.py")))
    return {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in paths
    }


def probabilities(response: dict, questions: dict) -> dict:
    if response.get("model") != MODEL:
        raise ValueError("response model mismatch")
    answers = response.get("answers", {})
    if set(answers) != set(questions):
        raise ValueError("answer keys mismatch")
    result = {}
    for arm in ("baseline", "korean"):
        answer = answers[arm]
        value = answer.get("noul")
        if answer.get("type") != "noul" or not valid_probability(value):
            raise ValueError("invalid noul")
        result[arm] = value
    answer = answers["choice"]
    distribution = answer.get("probabilities", {})
    if answer.get("type") != "choice" or set(distribution) != set(
        questions["choice"]["criteria"]
    ):
        raise ValueError("choice keys mismatch")
    if not all(valid_probability(p) for p in distribution.values()):
        raise ValueError("invalid choice probability")
    if (
        abs(sum(distribution.values()) - 1) > 0.01
        or answer.get("choice") not in distribution
    ):
        raise ValueError("invalid choice distribution")
    result["choice"] = distribution["persistent"]
    return result


def valid_probability(value: Any) -> bool:
    return type(value) in (float, int) and math.isfinite(value) and 0 <= value <= 1


# cost: io HTTPS 한 번과 원응답 한 번 저장; basis: estimate
def collect_one(case: dict, questions: dict, key: str, run_id: str) -> dict:
    request = {"model": MODEL, "state": case["state"], "questions": questions}
    text = json.dumps(request, ensure_ascii=False).replace(key, "[secret]")
    text = re.sub(r"apikey_[A-Za-z0-9_]+", "[secret]", text)
    data = text.encode()
    record = {
        "run_id": run_id,
        "sample_id": case["sample_id"],
        "ts_utc": now(),
        "request": json.loads(text),
        "request_sha256": hashlib.sha256(data).hexdigest(),
    }
    started = time.monotonic()
    try:
        request = urllib.request.Request(
            ENDPOINT,
            data=data,
            method="POST",
            headers={
                "Authorization": "Bearer " + key,
                "Content-Type": "application/json",
            },
        )
        with urllib.request.build_opener(NoRedirect).open(
            request, timeout=30
        ) as response:
            raw = response.read().decode().replace(key, "[secret]")
        raw = re.sub(r"apikey_[A-Za-z0-9_]+", "[secret]", raw)
        record["response"] = json.loads(raw)
        record["probabilities"] = probabilities(record["response"], questions)
        record["status"] = "ok"
    except urllib.error.HTTPError as error:
        record.update(status="http_error", http_status=error.code)
    except (ValueError, KeyError, TypeError, AttributeError) as error:
        record.update(status="schema_error", error_type=type(error).__name__)
    except OSError as error:
        record.update(status="transport_unknown", error_type=type(error).__name__)
    record["latency_s"] = time.monotonic() - started
    save_new(PRIVATE / "raw" / (case["sample_id"] + ".json"), record)
    return record


def seal() -> dict:
    path = PRIVATE / "design-seal.json"
    if path.exists():
        value = json.loads(path.read_text())
        if value["hashes"] != hashes():
            raise RuntimeError("sealed files changed")
        return value
    value = {
        "ts_utc": now(),
        "hashes": hashes(),
        "commit_created": False,
        "reason": "user instruction prohibits commits",
    }
    save_new(path, value)
    return value


# cost: io 인증 GET 한 번과 최대 4000번 추론; basis: estimate
def main() -> None:
    os.umask(0o077)
    subprocess.run(["git", "check-ignore", "-q", str(PRIVATE)], cwd=ROOT, check=True)
    cases = json.loads((PRIVATE / "samples.json").read_text())
    if not cases or len(cases) > 4000:
        raise RuntimeError("invalid sample count")
    questions = json.loads((PUBLIC / "questions.json").read_text())
    plan = seal()
    run_id = hashlib.sha256(json.dumps(plan, sort_keys=True).encode()).hexdigest()[:16]
    (PRIVATE / "raw").mkdir(exist_ok=True)
    env = PRIVATE / "env.json"
    if not env.exists():
        save_new(
            env,
            {
                "run_id": run_id,
                "python": platform.python_version(),
                "os": platform.platform(),
                "machine": platform.machine(),
                "model": MODEL,
                "ts_utc": now(),
                "seed": 38220261005,
            },
        )
    key = getpass.getpass("Jev test key: ")
    if not key:
        raise RuntimeError("credential missing")
    auth_request = urllib.request.Request(
        "https://api.typesafe.ai/v1/models", headers={"Authorization": "Bearer " + key}
    )
    try:
        with urllib.request.build_opener(NoRedirect).open(
            auth_request, timeout=30
        ) as response:
            json.loads(response.read())
    except urllib.error.HTTPError as error:
        print(
            json.dumps({"authentication": "failed", "http_status": error.code}),
            flush=True,
        )
        return
    print(
        json.dumps({"authentication": "ok", "run_id": run_id, "samples": len(cases)}),
        flush=True,
    )
    reserved_path = PRIVATE / "calls.jsonl"
    reserved = (
        {
            json.loads(line)["sample_id"]
            for line in reserved_path.read_text().splitlines()
        }
        if reserved_path.exists()
        else set()
    )
    completed = [json.loads(p.read_text()) for p in (PRIVATE / "raw").glob("*.json")]
    tokens = sum(
        r.get("response", {}).get("usage", {}).get("input_tokens", 0) for r in completed
    )
    pending = [case for case in cases if case["sample_id"] not in reserved]
    failures = 0
    stop = False
    start = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        with reserved_path.open("a") as ledger:
            for offset in range(0, len(pending), 4):
                if tokens * 0.042 / 1e6 >= 3:
                    break
                batch = pending[offset : offset + 4]
                for case in batch:
                    ledger.write(
                        json.dumps(
                            {
                                "run_id": run_id,
                                "sample_id": case["sample_id"],
                                "ts_utc": now(),
                            }
                        )
                        + "\n"
                    )
                ledger.flush()
                os.fsync(ledger.fileno())
                records = list(
                    pool.map(
                        lambda case: collect_one(case, questions, key, run_id), batch
                    )
                )
                completed.extend(records)
                for record in records:
                    tokens += (
                        record.get("response", {})
                        .get("usage", {})
                        .get("input_tokens", 0)
                    )
                    failures = 0 if record["status"] == "ok" else failures + 1
                    if record.get("http_status") in (401, 403) or failures >= 3:
                        stop = True
                if len(completed) % 200 == 0 or stop or offset + 4 >= len(pending):
                    print(
                        json.dumps(
                            {
                                "completed": len(completed),
                                "ok": sum(r["status"] == "ok" for r in completed),
                                "elapsed_s": round(time.monotonic() - start, 1),
                                "estimated_usd": round(tokens * 0.042 / 1e6, 4),
                                "stopped": stop,
                            }
                        ),
                        flush=True,
                    )
                if stop:
                    break
    key = ""
    print(json.dumps({"finished": len(completed), "planned": len(cases)}), flush=True)


if __name__ == "__main__":
    main()
