"""기존 실험의 호출 경계를 재사용하고 이번 실행의 원자료를 분리한다."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/jev-role-efficiency"
SEED = 534106
MODELS = {
    "opus": ("claude", "opus"),
    "sol": ("codex", "gpt-6-sol"),
    "astra": ("codex", "gpt-6-astra"),
    "sonnet": ("claude", "sonnet"),
    "haiku": ("claude", "haiku"),
    "luna": ("codex", "gpt-6-luna"),
    "terra": ("codex", "gpt-5.6-terra"),
}
BASELINE = {
    "design": "opus",
    "review": "sol",
    "implementation": "sonnet",
    "mechanical": "haiku",
}
spec = importlib.util.spec_from_file_location(
    "shared_calls", PUBLIC.parent / "target-model-choice/scripts/runtime.py"
)
BASE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(BASE)
BASE.PRIVATE = PRIVATE
BASE.LIMITS = {"claude": 90, "codex": 50, "jev": 100}
read, write, rows, digest = BASE.read, BASE.write, BASE.rows, BASE.digest
call_cli, call_jev, response_text, parse_json = (
    BASE.call_cli,
    BASE.call_jev,
    BASE.response_text,
    BASE.parse_json,
)


def setup() -> None:
    os.umask(0o077)
    BASE.setup()


def llm(model: str, prompt: str, trial: str) -> dict:
    kind, name = MODELS[model]
    record = call_cli(kind, name, prompt, trial)
    text, meta = response_text(record)
    return {
        "trial_id": trial,
        "value": parse_json(text),
        "meta": meta,
        "status": record["status"],
    }


def safe_state(value: Any) -> str:
    forbidden = {"expected", "tests", "gold", "future", "outcome"}

    def inspect(item: Any) -> None:
        if isinstance(item, dict):
            if forbidden.intersection(item):
                raise ValueError("outcome leakage in decision input")
            for child in item.values():
                inspect(child)
        elif isinstance(item, list):
            for child in item:
                inspect(child)

    inspect(value)
    return json.dumps(value, ensure_ascii=False)


def choice(instructions: str, options: list[str]) -> dict:
    return {
        "type": "choice",
        "instructions": instructions,
        "criteria": dict.fromkeys(options),
    }


def noul(instructions: str) -> dict:
    return {"type": "noul", "instructions": instructions}


def judge(state: dict, questions: dict, trial: str) -> dict:
    body = {"model": "jev-1.13.0", "state": safe_state(state), "questions": questions}
    record = call_jev(body, trial)
    if record.get("http_status") in (401, 403):
        raise RuntimeError("judge authentication rejected")
    return record
