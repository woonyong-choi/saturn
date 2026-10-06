"""원응답에서 trial을 다시 만든다. 가공, 검증, 분석이 같은 함수를 쓴다."""

from __future__ import annotations

import hashlib
import json
from datetime import datetime, timedelta

import fixtures
import selection
from runtime import MODELS, POLICY_VERSION, SEED, contract, parse_json, response_text

Raw = dict  # raw/<이름>.json -> 바이트


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_raw(directory) -> Raw:
    return {f"raw/{p.name}": p.read_bytes() for p in sorted(directory.glob("*.json"))}


def rec(raw: Raw, trial: str) -> dict | None:
    data = raw.get(f"raw/{trial}.json")
    return json.loads(data) if data is not None else None


def usage_entry(call_id: str, role: str, record: dict | None) -> dict:
    values = dict.fromkeys(contract.USAGE_FIELDS)
    if record and record.get("status") == "ok":
        if record["kind"] == "jev":
            usage = record.get("response", {}).get("usage", {})
            names = {"input_tokens": "input_tokens", "output_tokens": "output_tokens"}
        else:
            _, meta = response_text(record)
            usage = meta.get("usage", {})
            names = {
                "input_tokens": "input_tokens",
                "cache_creation_input_tokens": "cache_creation_input_tokens",
                "cache_read_input_tokens": "cache_read_input_tokens" if record["kind"] == "claude" else "cached_input_tokens",
                "output_tokens": "output_tokens",
            }
        for field, source in names.items():
            v = usage.get(source) if isinstance(usage, dict) else None
            values[field] = v if isinstance(v, int) and not isinstance(v, bool) else None
    return dict(call_id=call_id, role=role, parent_call_id=None, includes_children=False, **values)


def span(records: list[dict]) -> tuple[str | None, str | None, float | None]:
    spans, total = [], 0.0
    for r in records:
        if r and r.get("ts_utc") and isinstance(r.get("latency_s"), (int, float)):
            start = datetime.fromisoformat(r["ts_utc"])
            spans.append((start, start + timedelta(seconds=r["latency_s"])))
            total += r["latency_s"]
    if not spans:
        return None, None, None
    return min(s for s, _ in spans).isoformat(), max(e for _, e in spans).isoformat(), round(total, 6)


def build_trial(case: dict, provider: str, arm: str, rep: int, raw: Raw, tau: float, grader=selection.grade) -> dict:
    tid = case["task_id"]
    jev = [rec(raw, f"jev-{tid}-p{n}") for n in range(len(selection.jev_pieces(case)))] if arm == "jev" else None
    sel_name = f"{provider}-sel-{tid}-r{rep}"
    sel = rec(raw, sel_name) if arm == "llm" else None
    sel_text = response_text(sel)[0] if sel else None
    run_name = f"{provider}-run-{arm}-{tid}-r{rep}"
    run = rec(raw, run_name)
    chosen = selection.select(arm, case, fixtures.BUDGET_BYTES, tau, jev, sel_text)
    names = [f"raw/jev-{tid}-p{n}.json" for n in range(len(jev or []))] + ([f"raw/{sel_name}.json"] if sel else []) + ([f"raw/{run_name}.json"] if run else [])
    status, answer = "incomplete", None
    if run is not None:
        s = run.get("status")
        answer = parse_json(response_text(run)[0])
        status = "ok" if s == "ok" else "delivery_unknown" if s == "timeout" else "failed"
    tag = f"@{provider}-r{rep}-{arm}"  # 선택 호출은 이 trial이 쓴 만큼 귀속한다(같은 호출을 여러 trial이 쓰면 호출 ID를 달리해 두 번 세지 않게 한다)
    usage = [usage_entry(f"jev-{tid}-p{n}{tag}", "jev", r) for n, r in enumerate(jev or [])]
    if sel:
        usage.append(usage_entry(sel_name, "selector", sel))
    if run:
        usage.append(usage_entry(run_name, "parent", run))
    started, ended, latency = span([r for r in (*(jev or []), sel, run) if r])
    support = case["support_ids"]
    size = {b["id"]: len(b["text"].encode()) for b in case["blocks"]}
    files = [dict(name=n, sha256=sha(raw[n])) for n in names]
    meta = response_text(run)[1] if run and status == "ok" else {}
    reported = sorted(meta.get("modelUsage", {})) if isinstance(meta, dict) else []
    return dict(
        task_id=tid, family_id=case["family_id"], split=case["split"], stratum=case["stratum"], lang=case["lang"],
        length=case["length"], fixture_hash=case["fixture_hash"], arm=arm, rep=rep, seed=SEED, provider=provider,
        model=MODELS[provider], model_version=reported[0] if reported else None, policy_version=POLICY_VERSION, tau=tau,
        input_id=None, run_id=None, session_id=None,
        decision_id=run_name, started_at=started, ended_at=ended, latency_s=latency, raw_files=files,
        raw_sha256=sha("\n".join(f"{f['name']}:{f['sha256']}" for f in files).encode()),
        selection=dict(chosen, bytes=sum(size[i] for i in chosen["ids"]),
                       support_recall=(sum(s in chosen["ids"] for s in support) / len(support)) if support else None,
                       candidate_hash=sha(json.dumps([b["id"] for b in case["blocks"]]).encode())),
        answer=answer, answer_lenient=selection.lenient_json(response_text(run)[0]) if run else None,
        check=dict(grader="exact-v1", success=bool(status == "ok" and grader(case, answer)),
                   success_lenient=bool(status == "ok" and grader(case, selection.lenient_json(response_text(run)[0]) if run else None))),
        status=status, usage=usage,
    )


def build_trials(cases: list[dict], providers: dict[str, set], raw: Raw, tau: float, grader=selection.grade) -> list[dict]:
    return [
        build_trial(c, p, a, r, raw, tau, grader)
        for c in cases for p in ("claude", "codex") if c["task_id"] in providers[p]
        for r in range(3) for a in selection.ARMS
    ]


def check_leakage(raw: Raw) -> None:
    for name, data in raw.items():
        record = json.loads(data)
        if record.get("kind") == "jev":
            from runtime import safe_state
            safe_state(json.loads(record["request"]["state"]))
            safe_state(record["request"]["questions"])


def check_dataset(cases: list[dict], providers: dict[str, set], recorded: list[dict], raw: Raw, tau: float, grader=selection.grade) -> None:
    selection.grader_selftest(grader, cases)
    keys = {(t["task_id"], t["provider"], t["arm"], t["rep"]) for t in recorded}
    want = {(c["task_id"], p, a, r) for c in cases for p in providers if c["task_id"] in providers[p] for a in selection.ARMS for r in range(3)}
    if len(keys) != len(recorded) or keys != want:
        raise contract.ContractError("trial set does not match the sample")
    for t in recorded:
        contract.validate_trial(t)
        for f in t["raw_files"]:
            if f["name"] not in raw or sha(raw[f["name"]]) != f["sha256"]:
                raise contract.ContractError("raw hash mismatch: " + f["name"])
        contract.merge_usage(t["usage"])
        if t["status"] == "ok" and not any(e["role"] == "parent" for e in t["usage"]):
            raise contract.ContractError("ok trial without parent usage: " + t["decision_id"])
    contract.check_cross_trial_usage(recorded)
    check_leakage(raw)
    by_id = {c["task_id"]: c for c in cases}
    rebuilt = {(t["task_id"], t["provider"], t["arm"], t["rep"]): t for t in build_trials(cases, providers, raw, tau, grader)}
    for t in recorded:
        key = (t["task_id"], t["provider"], t["arm"], t["rep"])
        if t != rebuilt[key]:
            fields = [k for k in rebuilt[key] if rebuilt[key][k] != t.get(k)]
            raise contract.ContractError(f"trial differs from raw responses: {key} {fields}")
        run = rec(raw, t["decision_id"])
        if run is not None:
            sent = {b["id"] for b in json.loads(run["prompt"].split("\nRecords: ", 1)[1])}
            if run["prompt"] != selection.executor_prompt(by_id[t["task_id"]], t["selection"]["ids"]) or sent != set(t["selection"]["ids"]):
                raise contract.ContractError("executed records differ from selection: " + t["decision_id"])
