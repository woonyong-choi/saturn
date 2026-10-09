"""실제 provider 기록의 시간과 완료 상태를 재생 입력에서 보존한다."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from typing import Any

import pytest

PREPARE_PATH = Path(__file__).resolve().parents[1] / "01-prepare.py"
SPEC = importlib.util.spec_from_file_location("prepare", PREPARE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("prepare module unavailable")
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


def message(role: str, text: str, **metadata: str) -> dict[str, Any]:
    return {
        "timestamp": "2026-10-01T11:17:31.502Z",
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": role,
            "content": [{"text": text}],
            **metadata,
        },
    }


@pytest.mark.parametrize("metadata", [{"phase": "final_answer"}, {"channel": "final"}])
def test_completed_response_preserves_time_and_end(
    tmp_path: Path, metadata: dict[str, str]
) -> None:
    path = tmp_path / "source.jsonl"
    rows = [
        message("user", "old instruction"),
        message("assistant", "recorded answer", **metadata),
    ]
    path.write_text("\n".join(json.dumps(row) for row in rows))
    turns = PREPARE.turns(path, str)
    assert turns[0]["at_ms"] == 1790853451502
    assert turns[0]["end"] == "Completed"
    assert turns[0]["input"] == "old instruction"
    assert turns[0]["answer"] == "recorded answer\n"


def test_partial_response_keeps_unknown_end(tmp_path: Path) -> None:
    path = tmp_path / "source.jsonl"
    rows = [
        message("user", "# AGENTS.md instructions"),
        message("user", "read this"),
        message("assistant", "reading", phase="commentary"),
    ]
    path.write_text("\n".join(json.dumps(row) for row in rows))
    turns = PREPARE.turns(path, str)
    assert len(turns) == 1
    assert turns[0]["end"] is None
    assert turns[0]["line"] == 2
