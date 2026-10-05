#!/usr/bin/env python3
"""원자료(gzip JSON)에서 trial 표를 다시 만든다. 같은 원자료는 같은 바이트를 낸다."""
from __future__ import annotations

import gzip
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tasks  # noqa: E402

HERE = Path(__file__).resolve().parent
DATA = HERE.parent / "data"
PRICES = json.loads((HERE / "prices.json").read_text(encoding="utf-8"))["models"]
APPLY_CONFIDENCE = 0.6


def price_of(model: str | None) -> dict | None:
    for entry in PRICES:
        if model and model.startswith(entry["prefix"]):
            return entry
    return None


def usage_totals(rows: list[dict]) -> dict:
    """session별 누적(`ThreadCumulative`)은 마지막 행, 그 밖은 행의 합. 비어 있는 값은 알려진 합과 따로 센다."""
    groups: dict[tuple, list[dict]] = {}
    for r in rows:
        groups.setdefault((r["session_id"], r["agent_id"], r["model"], r["scope"] == "ThreadCumulative"), []).append(r)
    fields = ("input_tokens", "cache_read_tokens", "cache_write_tokens", "output_tokens")
    total = dict.fromkeys(fields, 0)
    missing_rows = 0
    by_model: dict[str, dict] = {}
    for (_, _, model, cumulative), items in sorted(groups.items(), key=lambda kv: (kv[0][0], kv[0][1], str(kv[0][2]), kv[0][3])):
        items.sort(key=lambda r: r["id"])
        used = items[-1:] if cumulative else items
        slot = by_model.setdefault(str(model), dict.fromkeys(fields, 0))
        for r in used:
            if any(r[f] is None for f in fields):
                missing_rows += 1
            for f in fields:
                total[f] += r[f] or 0
                slot[f] += r[f] or 0
    return dict(total=total, by_model=by_model, rows=len(rows), missing_rows=missing_rows)


def cost_upper(by_model: dict) -> float | None:
    cost = 0.0
    for model, t in by_model.items():
        p = price_of(model)
        if p is None:
            return None
        cost += (t["input_tokens"] + t["cache_read_tokens"] + t["cache_write_tokens"]) * p["input"] / 1e6 + t["output_tokens"] * p["output"] / 1e6
    return round(cost, 6)


def target_answer(judgments: list[dict]) -> dict | None:
    for j in judgments:
        try:
            received = json.loads(j["received"]) if j.get("received") else {}
        except ValueError:
            continue
        answer = received.get("answers", {}).get("target_model")
        if isinstance(answer, dict):
            return dict(choice=answer.get("choice"), confidence=answer.get("confidence"), probabilities=dict(sorted((answer.get("probabilities") or {}).items())))
    return None


def shadow_answer(shadows: list[dict]) -> list | None:
    for s in shadows:
        try:
            return json.loads(s["candidates"])
        except (ValueError, KeyError):
            continue
    return None


def build_row(phase: str, data: dict, by_id: dict) -> dict:
    task = by_id[data["task_id"]]
    t = data["tables"]
    spec = data["arm_spec"]
    selections = t.get("model_selections") or []
    sel = selections[0] if len(selections) == 1 else None
    runs = t.get("runs") or []
    inputs = t.get("inputs") or []
    judgments = t.get("judgments") or []
    usage = usage_totals(t.get("usage") or [])
    ended = [r["ended_at"] for r in runs if r["ended_at"] is not None]
    accepted = [i["accepted_at"] for i in inputs]
    latency = round((max(ended) - min(accepted)) / 1000, 3) if ended and accepted else None
    router_in = sum(j["input_tokens"] or 0 for j in judgments)
    router_out = sum(j["output_tokens"] or 0 for j in judgments)
    target = target_answer(judgments)
    return dict(
        phase=phase, tid=data["tid"], task_id=task["task_id"], family_id=task["family_id"], split=task["split"], fixture_hash=task["fixture_hash"],
        difficulty=task["difficulty"], lang=task["lang"], ctx=task["ctx"], arm=data["arm"], layer=spec["layer"], mode=spec["mode"], default_model=spec["model"],
        status=data["status"], notes=data["notes"], retried=data.get("retried_after"), success=bool(data["check"]["success"]),
        passed=data["check"]["passed"], total_cases=data["check"]["total"], failed_cases=data["check"]["failed"],
        provider_requests=len(runs), router_calls=len(judgments), packets=len(t.get("handoff_packets") or []),
        selection=None if sel is None else dict(source=sel["source"], model=sel["model"], reason=sel["reason"], applied=bool(sel["applied"])),
        target=target,
        target_applied=bool(target and target["confidence"] is not None and target["confidence"] >= APPLY_CONFIDENCE),
        executed_models=sorted({str(r["model"]) for r in (t.get("usage") or [])}),
        sessions=[dict(provider=s["provider"], model=s["model"]) for s in (t.get("sessions") or [])],
        usage=usage, cost_usd_upper=cost_upper(usage["by_model"]) if usage["rows"] else None,
        router_tokens=dict(input=router_in, output=router_out), latency_s=latency, shadow=shadow_answer(t.get("model_shadows") or []),
    )


def main() -> None:
    out = DATA / "processed"
    out.mkdir(exist_ok=True)
    for phase in ("dev", "confirm"):
        files = sorted((DATA / "raw" / phase).glob("*.json.gz")) if (DATA / "raw" / phase).exists() else []
        if not files:
            continue
        by_id = {t["task_id"]: t for t in tasks.make_tasks(phase)}
        rows = []
        for f in files:
            with gzip.open(f, "rt", encoding="utf-8") as fh:
                rows.append(build_row(phase, json.load(fh), by_id))
        rows.sort(key=lambda r: r["tid"])
        (out / f"trials-{phase}.json").write_text(json.dumps(rows, ensure_ascii=False, sort_keys=True, indent=1) + "\n", encoding="utf-8")
        print(f"{phase}: {len(rows)} trials")


if __name__ == "__main__":
    main()
