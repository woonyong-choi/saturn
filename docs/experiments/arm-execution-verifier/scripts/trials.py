"""원응답에서 trial을 다시 만들고 기록된 trial과 대조한다."""

from __future__ import annotations

import copy
import hashlib
import json
from datetime import datetime, timedelta
from typing import Any, Callable

import arms
import contract
from arm_runtime import PRIVATE, SEED, POLICY_VERSION, jev

Raw = dict  # 비공개 폴더 기준 이름 -> 원응답 바이트


def raw_name(trial: str) -> str:
    return f"raw/{trial}.json"


def load_raw() -> Raw:
    return {
        f"raw/{p.name}": p.read_bytes() for p in sorted((PRIVATE / "raw").glob("*.json"))
    }


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _record(raw: Raw, trial: str) -> dict | None:
    data = raw.get(raw_name(trial))
    return json.loads(data) if data is not None else None


def _usage_entry(call_id: str, role: str, record: dict | None) -> dict:
    values = dict.fromkeys(contract.USAGE_FIELDS)
    if record and record.get("status") == "ok":
        if record["kind"] == "jev":
            usage = record.get("response", {}).get("usage", {})
            names = {"input_tokens": "input_tokens", "output_tokens": "output_tokens"}
        else:
            _, meta = jev.response_text(record)
            usage = meta.get("usage", {})
            names = {
                "input_tokens": "input_tokens",
                "cache_creation_input_tokens": "cache_creation_input_tokens",
                "cache_read_input_tokens": "cache_read_input_tokens"
                if record["kind"] == "claude"
                else "cached_input_tokens",
                "output_tokens": "output_tokens",
            }
        for field, source in names.items():
            v = usage.get(source) if isinstance(usage, dict) else None
            values[field] = v if isinstance(v, int) and not isinstance(v, bool) else None
    return dict(call_id=call_id, role=role, parent_call_id=None, includes_children=False, **values)


def _times(records: list[dict]) -> tuple[str | None, str | None]:
    spans = []
    for r in records:
        if r and r.get("ts_utc") and isinstance(r.get("latency_s"), (int, float)):
            start = datetime.fromisoformat(r["ts_utc"])
            spans.append((start, start + timedelta(seconds=r["latency_s"])))
    if not spans:
        return None, None
    return min(s for s, _ in spans).isoformat(), max(e for _, e in spans).isoformat()


def _latency(records: list[dict]) -> float | None:
    values = [r["latency_s"] for r in records if isinstance(r.get("latency_s"), (int, float))]
    return round(sum(values), 6) if values else None


def build_trial(case: dict, arm: str, raw: Raw, grader: Callable = arms.grade) -> dict:
    select_trial = f"select-{arm}-{case['task_id']}"
    run_trial = f"run-{arm}-{case['task_id']}"
    select_record = _record(raw, select_trial) if arm in ("llm", "jev") else None
    run_record = _record(raw, run_trial)
    selection = arms.select(arm, case, select_record)
    names = ([raw_name(select_trial)] if select_record is not None else []) + (
        [raw_name(run_trial)] if run_record is not None else []
    )
    status = "incomplete"
    answer = None
    if run_record is not None:
        s = run_record.get("status")
        text, _ = jev.response_text(run_record)
        answer = jev.parse_json(text)
        status = "ok" if s == "ok" else "delivery_unknown" if s == "timeout" else "failed"
    usage = []
    if select_record is not None:
        usage.append(_usage_entry(select_trial, "jev" if arm == "jev" else "selector", select_record))
    if run_record is not None:
        usage.append(_usage_entry(run_trial, "parent", run_record))
    started, ended = _times([r for r in (select_record, run_record) if r])
    kind = (run_record or {}).get("kind")
    meta = jev.response_text(run_record)[1] if run_record and status == "ok" else {}
    reported = sorted(meta.get("modelUsage", {})) if isinstance(meta, dict) else []
    files = [dict(name=n, sha256=sha(raw[n])) for n in names]
    return dict(
        task_id=case["task_id"],
        family_id=case["family_id"],
        split=case["split"],
        fixture_hash=case["fixture_hash"],
        arm=arm,
        seed=SEED,
        provider=kind,
        model=arms.EXECUTOR,
        model_version=reported[0] if reported else None,
        policy_version=POLICY_VERSION,
        input_id=None,
        run_id=None,
        session_id=None,
        decision_id=f"{case['task_id']}-{arm}",
        started_at=started,
        ended_at=ended,
        latency_s=_latency([r for r in (select_record, run_record) if r]),
        raw_files=files,
        raw_sha256=sha("\n".join(f"{f['name']}:{f['sha256']}" for f in files).encode()),
        selection=dict(selection, candidate_hash=sha(json.dumps(arms.block_ids(case)).encode())),
        answer=answer,
        check=dict(grader="exact-v1", success=bool(status == "ok" and grader(case, answer))),
        status=status,
        usage=usage,
    )


def build_trials(cases: list[dict], raw: Raw, grader: Callable = arms.grade) -> list[dict]:
    return [build_trial(c, a, raw, grader) for c in cases for a in contract.ARMS]


def check_leakage(raw: Raw) -> None:
    for name, data in raw.items():
        record = json.loads(data)
        if record.get("kind") == "jev":
            jev.safe_state(json.loads(record["request"]["state"]))


def check_prompts(cases: list[dict], recorded: list[dict], raw: Raw) -> None:
    """실제 실행 요청에 들어간 기록 블록이 선택 ID와 같아야 한다."""
    by_id = {c["task_id"]: c for c in cases}
    for t in recorded:
        record = _record(raw, f"run-{t['arm']}-{t['task_id']}")
        if record is None:
            continue
        sent = {b["id"] for b in json.loads(record["prompt"].split("\nRecords: ", 1)[1])}
        if record["prompt"] != arms.executor_prompt(by_id[t["task_id"]], t["selection"]["ids"]) or sent != set(t["selection"]["ids"]):
            raise contract.ContractError("executed records differ from selection: " + t["decision_id"])


def check_dataset(cases: list[dict], recorded: list[dict], raw: Raw, grader: Callable = arms.grade) -> None:
    """원자료 해시, 계약, 선택 재계산, 채점, 사용량, 정답 누출을 모두 확인한다."""
    arms.grader_selftest(grader, cases)
    index = {(t.get("task_id"), t.get("arm")): t for t in recorded}
    if len(index) != len(recorded) or set(index) != {(c["task_id"], a) for c in cases for a in contract.ARMS}:
        raise contract.ContractError("trial set does not match the sample")
    for t in recorded:
        contract.validate_trial(t)
        for f in t["raw_files"]:
            if f["name"] not in raw or sha(raw[f["name"]]) != f["sha256"]:
                raise contract.ContractError("raw hash mismatch: " + f["name"])
        contract.validate_usage(t["usage"])
        if t["status"] == "ok" and not t["usage"]:
            raise contract.ContractError("ok trial without usage: " + t["decision_id"])
        contract.merge_usage(t["usage"])
    contract.check_cross_trial_usage(recorded)
    check_leakage(raw)
    check_prompts(cases, recorded, raw)
    rebuilt = {(t["task_id"], t["arm"]): t for t in build_trials(cases, raw, grader)}
    for key, t in index.items():
        if t != rebuilt[key]:
            fields = [k for k in rebuilt[key] if rebuilt[key][k] != t.get(k)]
            raise contract.ContractError(f"trial differs from raw responses: {key} {fields}")
    by_task: dict[str, set] = {}
    for t in recorded:
        by_task.setdefault(t["task_id"], set()).add((t["fixture_hash"], t["selection"]["candidate_hash"]))
    if any(len(v) != 1 for v in by_task.values()):
        raise contract.ContractError("arms of a task do not share the candidate set")
