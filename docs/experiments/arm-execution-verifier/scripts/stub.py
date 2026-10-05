"""모델을 부르지 않는 결정적 호출기. 파이프라인과 검증기 확인용이며 어떤 효과의 근거도 아니다."""

from __future__ import annotations

import hashlib
import json
import re
from datetime import datetime, timedelta, timezone

from arm_runtime import PRIVATE, jev, read, write

_BASE = datetime(2026, 1, 1, tzinfo=timezone.utc)
_clock = {"n": 0}


def _stamp() -> tuple[str, float]:
    _clock["n"] += 1
    return (_BASE + timedelta(seconds=7 * _clock["n"])).isoformat(), 0.25 + (_clock["n"] % 5) / 10


def _every(trial: str, modulus: int, salt: str) -> bool:
    return int(hashlib.sha256((salt + trial).encode()).hexdigest()[:8], 16) % modulus == 0


def _existing(trial: str) -> dict | None:
    path = PRIVATE / "raw" / (trial + ".json")
    return read(path) if path.exists() else None


def judge(state: dict, questions: dict, trial: str) -> dict:
    if (found := _existing(trial)) is not None:
        return found
    jev.safe_state(state)
    ts, latency = _stamp()
    record = dict(
        trial_id=trial,
        kind="jev",
        ts_utc=ts,
        request=dict(state=json.dumps(state), questions=questions),
        latency_s=latency,
    )
    if _every(trial, 7, "jev-fail"):
        record.update(status="failed", error_type="StubError")
    else:
        project = re.search(r"fam540-\d+", state["task"]).group(0)
        low = _every(trial, 11, "jev-low")
        answers = {}
        for i, b in enumerate(state["blocks"]):
            hit = 0.6 if f"Port for {project} was" in b["text"] else 0.1 if "unrelated service" in b["text"] else 0.9
            score = (0.3 if low else hit) - i * 0.001
            answers[b["id"]] = {"noul": score}
        usage = dict(input_tokens=len(json.dumps(state)) // 4, output_tokens=40)
        record.update(
            status="ok", http_status=200, response=dict(answers=answers, usage=usage)
        )
    write(PRIVATE / "raw" / (trial + ".json"), record)
    return record


def _answer(prompt: str, trial: str) -> str:
    project = re.search(r"fam540-\d+", prompt).group(0)
    if trial.startswith("select-llm-"):
        if _every(trial, 6, "llm-bad"):
            return "not json"
        blocks = json.loads(prompt.split("data. No tools. ", 1)[1])["blocks"]
        hits = [b["id"] for b in blocks if "unrelated service" not in b["text"] and "was" not in b["text"].split(".")[0].split()]
        return json.dumps({"selected": hits[:3]})
    text = " ".join(b["text"] for b in json.loads(prompt.split("\nRecords: ", 1)[1]))
    out: dict = {}
    if '"region"' in prompt:
        region = re.search(rf"Current region for {project} is (\w+)", text)
        token = re.search(rf"Current token for {project} is (\w+)", text)
        out["region"] = region.group(1) if region else None
        out["token"] = token.group(1) if token else None
    port = re.search(r"changed to (\d+)", text) or re.search(rf"Port for {project} was (\d+)", text)
    out["port"] = int(port.group(1)) if port else None
    return json.dumps(out)


def llm(model: str, prompt: str, trial: str) -> dict:
    record = _existing(trial)
    if record is None:
        ts, latency = _stamp()
        record = dict(
            trial_id=trial, kind="claude", model=model, ts_utc=ts, prompt=prompt, latency_s=latency
        )
        if trial.startswith("run-") and _every(trial, 17, "exec-timeout"):
            record["status"] = "timeout"
        else:
            usage = dict(
                input_tokens=len(prompt) // 4,
                cache_read_input_tokens=0,
                cache_creation_input_tokens=0,
                output_tokens=30,
            )
            if trial.startswith("run-") and _every(trial, 13, "usage-null"):
                usage.pop("cache_read_input_tokens")
            envelope = dict(
                result=_answer(prompt, trial), is_error=False, usage=usage, modelUsage={model: {}}
            )
            record.update(status="ok", returncode=0, stdout=json.dumps(envelope), stderr="")
        write(PRIVATE / "raw" / (trial + ".json"), record)
    text, meta = jev.response_text(record)
    return dict(trial_id=trial, value=jev.parse_json(text), meta=meta, status=record["status"])
