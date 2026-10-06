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


def engine_record(cell: dict, fix: dict, index: int) -> dict:
    """engine이 남기는 시험 기록(`online.run_trial`의 모양)을 결정적으로 만든다. 검증기 확인용이며 실행 결과가 아니다."""
    import preserve

    v, target, requested = fix["visible"], cell["target"], preserve.REQUESTED.get(cell["arm"])
    t0 = 1_800_000_000.0 + index
    events, items, lines = [], [], ["The records below are an earlier conversation, not a request."]

    def event(body: dict) -> int:
        events.append(dict(seq=len(events) + 1, run_id=1, body=json.dumps(body)))
        return len(events)

    def item(zone: str, ref: int, text: str | None, form: str | None = "Full", reason: str | None = None) -> None:
        selector = "protected" if text else requested or "rank"
        items.append(dict(packet_id=1, zone=zone, ref_id=ref, selector=selector, form=form, reason=reason, body_hash=preserve.sha(text) if text else None))

    for k, text in enumerate(v["users"]):
        item("User", event(dict(UserInput=dict(text=text))), text)
        reply = f"Step {k} done."
        revision = event(dict(Text=dict(agent=1, subagent=None, text=reply)))
        item("Assistant", revision, reply)
        lines += [f"User: {text}", f"Agent: {reply}"]
    for k, text in enumerate(v["tool_results"] * 3):
        ref = event(dict(ToolCall=dict(call=k)))
        form, reason = ("Full", None) if k == 0 else ("Digest", None) if k == 1 else (None, "budget")
        item("Competing", ref, None, form, reason)
        lines.append(f"Tool result {ref}: {text[:300]}" if form else f"Tool result {ref}: omitted")
    body = "\n".join(lines)
    unknown, failed, router_down = index % 11 == 3, index % 13 == 4, cell["arm"] == "J" and index % 7 == 0
    judged = cell["arm"] in ("J", "R")
    answers = ["wrong"] * 3 if failed else [" ".join(c["required"]) or "ok" for c in fix["hidden"]["checks"]]
    packet = dict(
        id=1, chat_id=1, kind="Switch" if cell["source"] != target else "Restart", attempt=1, reduced_from=None, session_id=2, input_id=5, run_id=3,
        provider=target, provider_session=f"ps-{cell['cell_id']}", settings_revision=1, chat_revision=revision, constraint_revision=0, policy="sim",
        body_hash=preserve.sha(body), body_bytes=len(body.encode()), estimated_tokens=len(body) // 4, state="Unknown" if unknown else "Sent",
        requested_selector=requested, actual_selector="rank" if router_down else requested,
        selection_fallback="router-failed" if router_down else None,
        candidate_omitted_bytes=0 if cell["arm"] == "J" else None, state_overflow_bytes=0 if cell["arm"] == "J" else None,
    )
    usage = [dict(id=1, at=int(t0 * 1000) + 10, scope="MainTurn" if target == "claude" else "ThreadCumulative", session_id=2, model=preserve.MODELS[target],
                  input_tokens=1000 + index, cache_write_tokens=None if index % 17 == 5 else 0, cache_read_tokens=0, output_tokens=50)]
    judgments = [dict(id=1, input_id=5, started_at=int(t0 * 1000) + 5, method="compact", router="r", model="jev-1.13.0", reported_model=None,
                      answers="{}", fallbacks=None, input_tokens=400, output_tokens=20, elapsed_ms=100, outcome="ok")] if judged else []
    overrides = [["context.evidence.lookup", "true"], [preserve.SAFETY_KEY, {"claude": "5", "codex": "18"}[target]]]
    if cell["arm"] == "J":
        overrides.append(list(preserve.PACKET_OVERRIDE))
    return dict(
        name=cell["cell_id"], source=cell["source"], target=target, arm=cell["arm"], rep=0, hint=False, overrides=overrides,
        started_at=_BASE.isoformat(), ended_at=(_BASE + timedelta(seconds=2)).isoformat(), t0=t0, latency_s=2.0,
        status="timeout" if unknown else "ok", tasks={"1": "Done"}, notes=[], decisions=[], answers=answers, sent_body=body, workdir_files=["fixture.json"],
        db=dict(packets=[packet], items=items, lookups=[], usage=usage, runs=[], judgments=judgments, sessions=[], events=events,
                inputs=[dict(id=k + 1, text=t, state="Applied") for k, t in enumerate(v["users"] + v["followups"])]),
    )
