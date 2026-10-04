"""실패·경계값·형식 수리가 실험 점수를 왜곡하지 않는지 검사한다."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from metrics import counts, values  # noqa: E402
from protocol import parse  # noqa: E402


def test_failed_negative_is_not_counted_as_true_negative() -> None:
    data = [
        dict(gold="not_constraint", probability=None, valid=False),
        dict(gold="constraint", probability=0.8, valid=True),
    ]
    result = counts(data, 0.8)
    assert result == dict(tp=1, fp=0, tn=0, fn=0, failed=1, n=2, positives=1)
    assert values(data, 0.8)["accuracy"] == 0.5


def test_failed_positive_remains_in_recall_denominator() -> None:
    data = [
        dict(gold="constraint", probability=None, valid=False),
        dict(gold="constraint", probability=0.8, valid=True),
    ]
    assert values(data, 0.8)["recall"] == 0.5
    assert values(data, 0.8)["f1"] == 2 / 3


def codex_record(answer: str, extra_type: str) -> dict:
    events = [
        dict(type="item.completed", item=dict(type=extra_type)),
        dict(type="item.completed", item=dict(type="agent_message", text=answer)),
        dict(type="turn.completed", usage=dict(input_tokens=10, output_tokens=5)),
    ]
    return dict(
        kind="codex", status="ok", stdout="\n".join(json.dumps(e) for e in events)
    )


def test_non_tool_error_event_does_not_invalidate_answer() -> None:
    record = codex_record(
        '{"label":"constraint","reason":"뒤 작업에도 적용했다."}', "error"
    )
    assert parse(record, "gold")["valid"]


def test_actual_command_execution_invalidates_answer() -> None:
    record = codex_record(
        '{"is_constraint":true,"confidence":0.8}', "command_execution"
    )
    assert not parse(record, "query")["valid"]


def test_invalid_wrapped_json_keeps_usage_and_wrapper_evidence() -> None:
    record = codex_record("```json\n{not json}\n```", "error")
    result = parse(record, "query")
    assert not result["valid"]
    assert result["wrapped"]
    assert result["envelope"]["usage"]["input_tokens"] == 10


def test_boolean_disagreeing_with_probability_is_format_failure() -> None:
    record = codex_record('{"is_constraint":false,"confidence":0.8}', "error")
    assert not parse(record, "query")["valid"]


def test_failed_process_keeps_usage_without_accepting_answer() -> None:
    record = codex_record('{"is_constraint":true,"confidence":0.8}', "error")
    record["status"] = "process_error"
    result = parse(record, "query")
    assert not result["valid"]
    assert result["envelope"]["usage"]["input_tokens"] == 10


def test_seal_coverage_rejects_empty_manifest(monkeypatch, tmp_path) -> None:
    import seal
    import pytest

    monkeypatch.setattr(seal, "PRIVATE", tmp_path)
    (tmp_path / "sample-seal.json").write_text("{}")
    with pytest.raises(RuntimeError, match="seal coverage mismatch"):
        seal.check_seals()


def test_malformed_jev_structure_is_a_format_failure() -> None:
    from protocol import parse_jev

    for response in ([], {"answers": None}, {"answers": {"is_constraint": None}}):
        assert not parse_jev({"status": "ok", "response": response})["valid"]


def test_codex_cost_counts_cache_write_tokens_once() -> None:
    import importlib

    process = importlib.import_module("03-process")
    cost = process.estimate_cost(
        {"kind": "codex", "model": "gpt-6-astra"},
        {
            "usage": {
                "input_tokens": 100,
                "cached_input_tokens": 20,
                "cache_write_input_tokens": 10,
                "output_tokens": 5,
            }
        },
    )
    assert cost == (70 * 10 + 20 * 1 + 10 * 12.5 + 5 * 50) / 1e6
