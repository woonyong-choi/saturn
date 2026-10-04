"""봉인한 원응답을 다시 읽어 비공개 정규화 관측을 만든다."""

from __future__ import annotations

import json

from protocol import parse, parse_jev
from runtime import PRIVATE, read, rows, write
from seal import check_seals


def estimate_cost(record: dict, envelope: dict) -> float | None:
    usage = envelope.get("usage", {})
    if not usage:
        return None
    if record.get("kind") == "jev":
        tokens = usage.get("input_tokens", usage.get("prompt_tokens"))
        return tokens * 0.042 / 1e6 if tokens is not None else None
    model = record.get("model", "")
    if record.get("kind") == "codex":
        rates = {
            "gpt-6-astra": (10, 1, 50),
            "gpt-6-sol": (2, 0.2, 10),
            "gpt-5.6-terra": (2, 0.2, 12),
            "gpt-6-luna": (0.1, 0.01, 0.5),
        }
        if model not in rates or "input_tokens" not in usage:
            return None
        inp, cache, out = rates[model]
        cached = usage.get("cached_input_tokens", 0)
        written = usage.get("cache_write_input_tokens", 0)
        return (
            (usage["input_tokens"] - cached - written) * inp
            + cached * cache
            + written * inp * 1.25
            + usage.get("output_tokens", 0) * out
        ) / 1e6
    rates = {
        "claude-sonnet-5-5": (2, 0.2, 10),
        "claude-opus-5-5": (4, 0.2, 20),
        "claude-haiku-4-5-20251001": (1, 0.1, 5),
    }
    if model not in rates:
        return envelope.get("total_cost_usd")
    inp, cache, out = rates[model]
    creation = usage.get("cache_creation_input_tokens", 0)
    hour = usage.get("cache_creation", {}).get("ephemeral_1h_input_tokens", 0)
    cost = usage.get("input_tokens", 0) * inp + usage.get("output_tokens", 0) * out
    cost += (
        (creation - hour) * inp * 1.25
        + hour * inp * 2
        + usage.get("cache_read_input_tokens", 0) * cache
    )
    return cost / 1e6


def main() -> None:
    check_seals()
    cases = {c["sample_id"]: c for c in read(PRIVATE / "samples.json")}
    gold = {c["sample_id"]: c["label"] for c in read(PRIVATE / "gold.json")}
    reserved = {r["trial_id"] for r in rows(PRIVATE / "calls.jsonl")}
    expected = {
        f"query-{name}-{sample_id}-r{repeat}"
        for name in ("sonnet", "opus", "haiku", "astra", "sol", "terra", "luna", "jev")
        for sample_id in cases
        for repeat in ((1, 2, 3) if name == "jev" else (1,))
    }
    if not expected.issubset(reserved):
        raise RuntimeError("query collection incomplete; no partial comparison")
    if not all((PRIVATE / "raw" / (trial + ".json")).exists() for trial in expected):
        raise RuntimeError("query responses incomplete; no partial comparison")
    observations = []
    for call in rows(PRIVATE / "calls.jsonl"):
        trial = call["trial_id"]
        if not trial.startswith("query-"):
            continue
        _, model, sample_id, repetition = trial.split("-")
        path = PRIVATE / "raw" / (trial + ".json")
        record = read(path) if path.exists() else dict(status="incomplete")
        result = parse_jev(record) if model == "jev" else parse(record, "query")
        case = cases[sample_id]
        envelope = result["envelope"]
        probability = result["value"]["confidence"] if result["valid"] else None
        observations.append(
            dict(
                run_id="2438948",
                trial_id=trial,
                condition=model,
                ts_utc=call["ts_utc"],
                sample_id=sample_id,
                repeat=int(repetition[1:]),
                project_id=case["project_id"],
                input_kind=case["input_kind"],
                length_band=case["length_band"],
                gold=gold[sample_id],
                status=record["status"],
                valid=result["valid"],
                wrapped=result["wrapped"],
                multiline=result["multiline"],
                probability=probability,
                claimed_prediction=result["claimed_prediction"],
                format_reason=result["format_reason"],
                latency_s=record.get("latency_s"),
                cost_usd=estimate_cost(record, envelope),
                cli_cost_usd=envelope.get("total_cost_usd"),
                usage=envelope.get("usage", {}),
                actual_models=list(envelope.get("modelUsage", {}))
                if model != "jev"
                else [envelope.get("model")],
                tool_events=envelope.get("tool_events", 0),
            )
        )
    path = PRIVATE / "processed.jsonl"
    path.write_text(
        "".join(
            json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n"
            for row in observations
        )
    )
    write(PRIVATE / "processing.json", dict(observations=len(observations)))
    print(json.dumps(dict(processed=len(observations))))


if __name__ == "__main__":
    main()
