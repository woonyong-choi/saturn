"""독립 정답을 봉인한 뒤 같은 입력의 질문 조건과 반복을 측정한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import random
import sys
import time
import urllib.error
import urllib.request
from collections import Counter

from protocol import QUESTION, SCOPE_QUESTION, TASK_QUESTION
from runtime import (
    BASE,
    PRIVATE,
    PUBLIC,
    SEED,
    load_module,
    now,
    read,
    redact,
    reserve,
    setup,
    write,
)

AUDIT = load_module("audit", PUBLIC / "scripts/01-audit.py")


def check_seal(name: str) -> None:
    for filename, checksum in read(PRIVATE / name).items():
        if hashlib.sha256((PRIVATE / filename).read_bytes()).hexdigest() != checksum:
            raise RuntimeError("seal mismatch: " + filename)


def collect_gold(cases: list[dict]) -> None:
    if (PRIVATE / "gold-seal.json").exists():
        check_seal("gold-seal.json")
        return
    first = {model: AUDIT.label(cases, "gold", model) for model in ("sol", "astra")}
    by_model = {
        model: {r["sample_id"]: r["value"] for r in values}
        for model, values in first.items()
    }
    disagreements = [
        c
        for c in cases
        if not by_model["sol"][c["sample_id"]]
        or not by_model["astra"][c["sample_id"]]
        or by_model["sol"][c["sample_id"]]["label"]
        != by_model["astra"][c["sample_id"]]["label"]
    ]
    third = {
        r["sample_id"]: r["value"] for r in AUDIT.label(disagreements, "third", "astra")
    }
    result = []
    for case in cases:
        sid = case["sample_id"]
        votes = [by_model[m][sid] for m in ("sol", "astra")]
        if sid in third:
            votes.append(third[sid])
        counts = Counter(v["label"] for v in votes if v)
        winner, n = counts.most_common(1)[0] if counts else ("uncertain", 0)
        result.append(
            dict(
                sample_id=sid,
                label=winner if n >= 2 else "uncertain",
                judgments=votes,
                initially_agreed=sid not in third,
            )
        )
    write(PRIVATE / "gold.json", result)
    write(
        PRIVATE / "gold-seal.json",
        {"gold.json": hashlib.sha256((PRIVATE / "gold.json").read_bytes()).hexdigest()},
    )
    print(
        json.dumps(
            dict(
                phase="gold",
                n=len(result),
                third=len(third),
                labels=dict(Counter(r["label"] for r in result)),
            )
        ),
        flush=True,
    )


def request_body(case: dict, condition: str) -> dict:
    question = QUESTION if condition == "current" else SCOPE_QUESTION
    questions = {"is_constraint": dict(type="noul", instructions=question)}
    if condition == "scope_gate":
        questions["task_only"] = dict(type="noul", instructions=TASK_QUESTION)
    return dict(model="jev-1.13.0", state=case["state"], questions=questions)


# cost: io 1 HTTPS request, no retry; basis: estimate
def call_jev(job: tuple) -> None:
    case, condition, repeat = job
    trial = f"jev-{condition}-{case['sample_id']}-r{repeat}"
    target = PRIVATE / "raw" / (trial + ".json")
    body = request_body(case, condition)
    if target.exists():
        if read(target)["request"] != body:
            raise RuntimeError("saved request mismatch")
        return
    key = os.environ.get("SATURN_JUDGE_KEY")
    if not key:
        raise RuntimeError("judge key missing")
    if not reserve("jev", trial):
        return
    request = urllib.request.Request(
        "https://api.typesafe.ai/v1/systemone",
        data=redact(json.dumps(body, ensure_ascii=False)).encode(),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    record = dict(
        trial_id=trial,
        kind="jev",
        model="jev-1.13.0",
        ts_utc=now(),
        request=body,
        condition=condition,
        repeat=repeat,
        sample_id=case["sample_id"],
    )
    started = time.monotonic()
    try:
        with urllib.request.build_opener(BASE.NoRedirect).open(
            request, timeout=60
        ) as response:
            record.update(
                status="ok",
                response=json.loads(redact(response.read().decode())),
                http_status=response.status,
            )
    except urllib.error.HTTPError as error:
        record.update(
            status="http_error",
            http_status=error.code,
            response_text=redact(error.read().decode(errors="replace")),
        )
    except (OSError, ValueError) as error:
        record.update(status="failed", error_type=type(error).__name__)
    record["latency_s"] = time.monotonic() - started
    write(target, record)
    if record.get("http_status") in (401, 403):
        raise RuntimeError("judge authentication rejected")


def collect_queries(cases: list[dict]) -> None:
    check_seal("gold-seal.json")
    AUDIT.label(cases, "query", "astra")
    jobs = [
        (c, condition, repeat)
        for repeat in (1, 2, 3)
        for c in cases
        for condition in ("current", "scope", "scope_gate")
    ]
    random.Random(SEED).shuffle(jobs)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for index, _ in enumerate(pool.map(call_jev, jobs), 1):
            if index % 100 == 0:
                write(
                    PRIVATE / "progress.json",
                    dict(phase="jev", completed=index, total=len(jobs), ts_utc=now()),
                )
                print(
                    json.dumps(dict(phase="jev", completed=index, total=len(jobs))),
                    flush=True,
                )


def main() -> None:
    setup()
    check_seal("sample-seal.json")
    seal = read(PRIVATE / "design-seal.json")
    for path, checksum in seal["files"].items():
        if hashlib.sha256((PUBLIC / path).read_bytes()).hexdigest() != checksum:
            raise RuntimeError("preregistered file changed: " + path)
    cases = read(PRIVATE / "samples.json")
    phase = sys.argv[1] if len(sys.argv) > 1 else "all"
    if phase in ("gold", "all"):
        collect_gold(cases)
    if phase in ("query", "all"):
        collect_queries(cases)


if __name__ == "__main__":
    main()
