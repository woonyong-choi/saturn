#!/usr/bin/env python3
"""trial 표에서 설계의 가설별 집계를 만든다. 입력이 같으면 출력 바이트가 같다."""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import stats  # noqa: E402

DATA = Path(__file__).resolve().parents[1] / "data"
RESULTS = Path(__file__).resolve().parents[1] / "results"
COMPARISONS = [
    ("claude", "c-auto", "c-sonnet", "primary"), ("claude", "c-auto", "c-haiku", "reference"), ("claude", "c-auto", "c-opus", "reference"),
    ("codex", "x-auto", "x-astra", "primary"), ("codex", "x-auto", "x-luna", "reference"),
    ("both", "b-auto", "c-sonnet", "primary"), ("both", "b-auto", "x-luna", "reference"),
]
FIXED = {"claude": ["c-haiku", "c-sonnet", "c-opus"], "codex": ["x-luna", "x-astra"], "both": ["c-sonnet", "x-luna"]}
REPLAY_ARM = {"claude/haiku": "c-haiku", "claude/sonnet": "c-sonnet", "claude/opus": "c-opus", "codex/gpt-6-luna": "x-luna", "codex/gpt-6-astra": "x-astra"}
DELTA_COST = 0.10


def index(rows: list[dict]) -> dict:
    return {(r["task_id"], r["arm"]): r for r in rows}


def pair_stats(rows_by: dict, tasks: list[str], treat: str, control: str) -> dict:
    pairs = [(rows_by.get((t, treat)), rows_by.get((t, control))) for t in tasks]
    pairs = [(a, c) for a, c in pairs if a is not None and c is not None]
    loss = sum(1 for a, c in pairs if not a["success"] and c["success"])
    gain = sum(1 for a, c in pairs if a["success"] and not c["success"])
    n = len(pairs)
    cost_pairs = [(a["cost_usd_upper"], c["cost_usd_upper"]) for a, c in pairs if a["cost_usd_upper"] is not None and c["cost_usd_upper"]]
    rel = [a / c - 1 for a, c in cost_pairs]
    time_diff = [a["latency_s"] - c["latency_s"] for a, c in pairs if a["latency_s"] is not None and c["latency_s"] is not None]
    times_a = [a["latency_s"] for a, _ in pairs if a["latency_s"] is not None]
    times_c = [c["latency_s"] for _, c in pairs if c["latency_s"] is not None]
    cost_a = [a["cost_usd_upper"] for a, _ in pairs if a["cost_usd_upper"] is not None]
    cost_c = [c["cost_usd_upper"] for _, c in pairs if c["cost_usd_upper"] is not None]
    return dict(
        pairs=n, success_treat=sum(a["success"] for a, _ in pairs), success_control=sum(c["success"] for _, c in pairs),
        loss_pairs=loss, gain_pairs=gain, loss_upper95=stats.cp_upper(loss, n), mcnemar_loss_p=stats.mcnemar_one_sided(loss, gain),
        cost_rel_diff=stats.bootstrap_mean(rel), cost_pairs_used=len(rel), cost_pairs_excluded=n - len(rel),
        mean_cost_treat=round(sum(cost_a) / len(cost_a), 6) if cost_a else None, mean_cost_control=round(sum(cost_c) / len(cost_c), 6) if cost_c else None,
        time_diff_s=stats.bootstrap_mean(time_diff), p95_treat=stats.percentile(times_a, 0.95), p95_control=stats.percentile(times_c, 0.95),
        router_tokens_treat=sum(a["router_tokens"]["input"] + a["router_tokens"]["output"] for a, _ in pairs),
        router_tokens_control=sum(c["router_tokens"]["input"] + c["router_tokens"]["output"] for _, c in pairs),
        provider_tokens_treat=sum(sum(a["usage"]["total"].values()) for a, _ in pairs),
    )


def verdict(layer: str, ps: dict, discriminating: bool, delta: float, safety_ok: bool) -> dict:
    h1 = ps["pairs"] > 0 and ps["loss_pairs"] == 0 and ps["loss_upper95"] is not None and ps["loss_upper95"] <= 0.10
    cost = ps["cost_rel_diff"]
    h2 = bool(cost) and cost["hi"] <= -delta
    td = ps["time_diff_s"]
    h3 = bool(td) and td["hi"] <= 0 and ps["p95_treat"] is not None and ps["p95_control"] is not None and ps["p95_treat"] <= ps["p95_control"]
    worse_quality = ps["loss_pairs"] > ps["gain_pairs"] and ps["mcnemar_loss_p"] < 0.05
    worse_effort = (bool(cost) and cost["lo"] > 0) or (bool(td) and td["lo"] > 0)
    if h1 and h2 and h3 and safety_ok and discriminating:
        decision = "채택"
    elif worse_quality or worse_effort:
        decision = "기각"
    else:
        decision = "보류"
    return dict(h1=h1, h2=h2, h3=h3, h4=safety_ok, discriminating=discriminating, decision=decision)


def auto_report(rows: list[dict], arm: str) -> dict:
    auto = [r for r in rows if r["arm"] == arm]
    reasons: dict[str, int] = {}
    for r in auto:
        sel = r["selection"]
        key = "router" if sel and sel["source"] == "router" else (sel["reason"] if sel else "missing")
        reasons[key] = reasons.get(key, 0) + 1
    confidences = [r["target"]["confidence"] for r in auto if r["target"] and r["target"]["confidence"] is not None]
    return dict(
        inputs=len(auto), applied_router=sum(1 for r in auto if r["selection"] and r["selection"]["source"] == "router"), outcomes=dict(sorted(reasons.items())),
        target_answers=len(confidences), confidence_ge_0_6=sum(1 for c in confidences if c >= 0.6), confidence_max=max(confidences) if confidences else None,
        confidence_mean=round(sum(confidences) / len(confidences), 4) if confidences else None,
        executed=dict(sorted({m: sum(1 for r in auto if m in r["executed_models"]) for r in auto for m in r["executed_models"]}.items())),
    )


def strata(rows: list[dict]) -> dict:
    out: dict = {}
    for arm in sorted({r["arm"] for r in rows}):
        by = {}
        for key in ("lang", "ctx", "difficulty", "family_id"):
            groups: dict[str, list[dict]] = {}
            for r in rows:
                if r["arm"] == arm:
                    groups.setdefault(r[key], []).append(r)
            by[key] = {g: dict(n=len(v), success=sum(x["success"] for x in v)) for g, v in sorted(groups.items())}
        out[arm] = by
    return out


def replay(rows_by: dict, rows: list[dict], layer_arm: str, control: str) -> dict:
    """확신도 문턱을 없앴다고 보고 `target_model` 1순위를 적용한 반사실. 같은 과제의 그 모델 고정 팔 시도 결과를 쓴다(탐색)."""
    tasks = sorted({r["task_id"] for r in rows if r["arm"] == layer_arm})
    synthetic = {}
    skipped = 0
    for t in tasks:
        a = rows_by[(t, layer_arm)]
        choice = a["target"]["choice"] if a["target"] else None
        fixed = rows_by.get((t, REPLAY_ARM.get(choice, "")))
        if fixed is None:
            skipped += 1
            continue
        synthetic[(t, "replay")] = dict(fixed, router_tokens=a["router_tokens"])
        synthetic[(t, control)] = rows_by[(t, control)]
    both = {**synthetic}
    ps = pair_stats(both, tasks, "replay", control)
    ps["tasks_without_replayable_choice"] = skipped
    return ps


def shadow_vs_success(rows: list[dict]) -> dict:
    """오토 팔의 그림자 충분성 확률(후보별)과 같은 과제에서 그 모델 고정 팔의 실제 성공(탐색, 개발만)."""
    rows_by = index(rows)
    out: dict = {}
    for r in rows:
        if r["mode"] != "auto" or not r["shadow"]:
            continue
        for c in r["shadow"]:
            fixed = rows_by.get((r["task_id"], REPLAY_ARM.get(c["model"], "")))
            if fixed is not None:
                bucket = out.setdefault(c["model"], dict(success=[], failure=[]))
                bucket["success" if fixed["success"] else "failure"].append(c["probability"])
    return {m: {k: dict(n=len(v), mean=round(sum(v) / len(v), 4) if v else None) for k, v in b.items()} for m, b in sorted(out.items())}


def analyze(phase: str, rows: list[dict], delta: float) -> dict:
    rows_by = index(rows)
    tasks = sorted({r["task_id"] for r in rows})
    summary: dict = dict(phase=phase, trials=len(rows), tasks=len(tasks))
    summary["status"] = {arm: dict(sorted({s: sum(1 for r in rows if r["arm"] == arm and r["status"] == s) for s in {x["status"] for x in rows}}.items())) for arm in sorted({r["arm"] for r in rows})}
    summary["arm_success"] = {arm: dict(n=sum(1 for r in rows if r["arm"] == arm), success=sum(1 for r in rows if r["arm"] == arm and r["success"]),
                                        wilson=stats.wilson(sum(1 for r in rows if r["arm"] == arm and r["success"]), sum(1 for r in rows if r["arm"] == arm)),
                                        mean_cost=(lambda xs: round(sum(xs) / len(xs), 6) if xs else None)([r["cost_usd_upper"] for r in rows if r["arm"] == arm and r["cost_usd_upper"] is not None]),
                                        mean_latency_s=(lambda xs: round(sum(xs) / len(xs), 3) if xs else None)([r["latency_s"] for r in rows if r["arm"] == arm and r["latency_s"] is not None]))
                              for arm in sorted({r["arm"] for r in rows})}
    summary["calls"] = dict(provider_requests=sum(r["provider_requests"] for r in rows), router_calls=sum(r["router_calls"] for r in rows),
                            usage_missing_rows=sum(r["usage"]["missing_rows"] for r in rows), trials_without_usage=sum(1 for r in rows if r["usage"]["rows"] == 0),
                            unknown_price=sum(1 for r in rows if r["usage"]["rows"] and r["cost_usd_upper"] is None), packets=sum(r["packets"] for r in rows),
                            reruns=sum(1 for r in rows if r["provider_requests"] > 1), retried_before_delivery=sum(1 for r in rows if r["retried"]))
    safety = dict(permission_blocked=sum(1 for r in rows if any("permission" in n for n in r["notes"])), input_mismatch=sum(1 for r in rows if any("does not match" in n for n in r["notes"])))
    summary["safety"] = safety
    layers: dict = {}
    for layer, treat, control, role in COMPARISONS:
        fixed_tasks_discordant = 0
        for t in tasks:
            outcomes = {rows_by[(t, a)]["success"] for a in FIXED[layer] if (t, a) in rows_by}
            fixed_tasks_discordant += 1 if len(outcomes) > 1 else 0
        discriminating = fixed_tasks_discordant > 0
        ps = pair_stats(rows_by, tasks, treat, control)
        entry = layers.setdefault(layer, dict(auto=auto_report(rows, treat), comparisons={}, fixed_discordant_tasks=fixed_tasks_discordant))
        entry["comparisons"][control] = dict(role=role, stats=ps, verdict=verdict(layer, ps, discriminating, delta, safety["permission_blocked"] == 0 and safety["input_mismatch"] == 0))
        if role == "primary":
            entry["replay_top_choice"] = replay(rows_by, rows, treat, control)
    summary["layers"] = layers
    summary["strata"] = strata(rows)
    if phase == "dev":
        summary["shadow_vs_success"] = shadow_vs_success(rows)
    return summary


def main() -> None:
    RESULTS.mkdir(exist_ok=True)
    delta = DELTA_COST
    frozen = DATA.parent / "frozen.json"
    if frozen.exists():
        delta = json.loads(frozen.read_text(encoding="utf-8"))["delta_cost"]
    for phase in ("dev", "confirm"):
        path = DATA / "processed" / f"trials-{phase}.json"
        if not path.exists():
            continue
        rows = json.loads(path.read_text(encoding="utf-8"))
        (RESULTS / f"summary-{phase}.json").write_text(json.dumps(analyze(phase, rows, delta), ensure_ascii=False, sort_keys=True, indent=1) + "\n", encoding="utf-8")
        print(f"{phase}: summary written")


if __name__ == "__main__":
    main()
