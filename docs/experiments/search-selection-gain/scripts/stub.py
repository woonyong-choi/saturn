"""모델을 부르지 않는 결정적 호출기. 파이프라인 점검용이며 어떤 선택 방법의 근거도 아니다.

과제 문장으로 숨긴 기대 값과 근거 위치를 찾아 응답을 만든다. 근거 블록이 요청에 다 들어 있으면 맞는 답, 아니면 틀린 답이다.
"""

from __future__ import annotations

import functools
import json
import re
from datetime import datetime, timezone

import fixtures


@functools.lru_cache(maxsize=1)
def _cases() -> dict:
    return {c["task"]: c for c in fixtures.build()}


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _target(trial: str):
    import runtime

    return runtime.PRIVATE / "raw" / (trial + ".json")


def call_cli(kind: str, model: str, prompt: str, trial: str) -> dict:
    import runtime

    target = _target(trial)
    if target.exists():
        return runtime.read(target)
    case = next((c for t, c in _cases().items() if prompt.startswith("Select up to") is False and prompt.startswith(t)), None)
    if prompt.startswith("Select up to"):
        task = re.search(r'"request": "(.*?)", "records"', prompt, re.S).group(1)
        task = json.loads('"' + task + '"')
        case = _cases()[task]
        ids = list(case["support_ids"]) + [b["id"] for b in case["blocks"][:2] if b["id"] not in case["support_ids"]]
        text = json.dumps({"selected": ids[:5] or ["r0"]})
    else:
        sent = set(re.findall(r'"id": "(r\d+)"', prompt))
        have = set(case["support_ids"]) <= sent
        text = json.dumps(case["expected"] if have else {"value": "unknown"})
    usage = dict(input_tokens=len(prompt) // 4, output_tokens=12)
    if kind == "claude":
        out = json.dumps(dict(result=text, is_error=False, usage=dict(usage, cache_creation_input_tokens=0, cache_read_input_tokens=0), modelUsage={model: {}}))
    else:
        out = "\n".join([
            json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": text}}),
            json.dumps({"type": "turn.completed", "usage": dict(usage, cached_input_tokens=0)}),
        ])
    record = dict(trial_id=trial, kind=kind, model=model, ts_utc=_now(), prompt=prompt, status="ok", returncode=0, stdout=out, stderr="", latency_s=0.01 + len(prompt) / 1e7)
    runtime.write(target, record)
    return record


def call_jev(body: dict, trial: str) -> dict:
    import runtime

    target = _target(trial)
    if target.exists():
        return runtime.read(target)
    task = json.loads(body["state"])["request"]
    case = _cases()[task]
    answers = {}
    for key in body["questions"]:
        answers[key] = {"noul": 0.9 if key in case["support_ids"] else 0.1}
    record = dict(trial_id=trial, kind="jev", ts_utc=_now(), request=body, status="ok", http_status=200,
                  response=dict(answers=answers, usage=dict(input_tokens=sum(len(q["instructions"]) for q in body["questions"].values()) // 4, output_tokens=len(answers) * 3)), latency_s=0.05)
    runtime.write(target, record)
    return record
