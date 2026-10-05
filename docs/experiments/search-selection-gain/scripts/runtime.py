"""호출 경계. target-model-choice의 공식 CLI와 TypeSafe HTTPS 호출기를 그대로 쓰고 이번 실행의 원자료만 분리한다."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
CALLER = os.environ.get("SEL_CALLER", "live")
if CALLER not in ("stub", "live"):
    raise SystemExit("SEL_CALLER must be stub or live")
PRIVATE = ROOT / ".local/experiments/search-selection-gain" / CALLER
SEED = 541001
POLICY_VERSION = "search-selection-gain-1"
MODELS = {"claude": "haiku", "codex": "gpt-5.6-luna"}
LIMITS = {"claude": 3900, "codex": 900, "jev": 800}
WORKERS = {"claude": 8, "codex": 3}

_spec = importlib.util.spec_from_file_location("shared_calls", PUBLIC.parent / "target-model-choice/scripts/runtime.py")
BASE = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(BASE)
BASE.PRIVATE = PRIVATE
BASE.LIMITS = LIMITS
read, write, rows, digest = BASE.read, BASE.write, BASE.rows, BASE.digest
call_cli, call_jev, response_text, parse_json = BASE.call_cli, BASE.call_jev, BASE.response_text, BASE.parse_json

_spec2 = importlib.util.spec_from_file_location("arm_contract", PUBLIC.parent / "arm-execution-verifier/scripts/contract.py")
contract = importlib.util.module_from_spec(_spec2)
_spec2.loader.exec_module(contract)


if CALLER == "stub":
    import stub

    call_cli, call_jev = stub.call_cli, stub.call_jev


def setup() -> None:
    os.umask(0o077)
    BASE.setup()
    (PRIVATE / "raw").mkdir(parents=True, exist_ok=True)


def safe_state(value: Any) -> str:
    """정답과 근거 위치 키가 요청 상태에 들어가지 못하게 막는다."""
    forbidden = {"expected", "tests", "gold", "future", "outcome", "support_ids", "support"}

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


def jev_call(state: dict, questions: dict, trial: str) -> dict:
    record = call_jev({"model": "jev-1.13.0", "state": safe_state(state), "questions": questions}, trial)
    if record.get("http_status") in (401, 403):
        raise RuntimeError("judge authentication rejected")
    return record
