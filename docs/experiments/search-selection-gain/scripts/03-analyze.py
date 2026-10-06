"""사전 등록한 지표와 판정을 계산한다. 같은 입력이면 같은 바이트를 낸다."""

from __future__ import annotations

import importlib.util
import json
import math
import random
from statistics import mean, median

import selection
from lock import LOCK
from runtime import CALLER, PRIVATE, PUBLIC, SEED, contract, read

BOOT = 4000
RESULTS = PUBLIC / "results" if CALLER == "live" else PRIVATE / "results"


def wilson(values: list[bool]) -> dict:
    n, k = len(values), sum(values)
    if not n:
        return dict(k=0, n=0, rate=None, wilson95=None)
    p, z = k / n, 1.959963984540054
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return dict(k=k, n=n, rate=p, wilson95=[max(0, center - half), min(1, center + half)])


def total_tokens(t: dict) -> dict:
    m = contract.merge_usage(t["usage"])
    known = m["known"]
    selection_calls = [e for e in t["usage"] if e["role"] != "parent"]
    selection = sum(v or 0 for e in selection_calls for k, v in e.items() if k in contract.USAGE_FIELDS)
    return dict(all=sum(known.values()), new=known["input_tokens"] + known["cache_creation_input_tokens"] + known["output_tokens"],
                selection=selection, unreported=m["unreported_calls"])


def per_task(items: list[dict], f) -> dict[str, float]:
    """과제(계열) 하나를 단위로 반복 3회의 평균을 낸다."""
    groups: dict[str, list[float]] = {}
    for t in items:
        groups.setdefault(t["task_id"], []).append(f(t))
    return {k: mean(v) for k, v in groups.items()}


def boot_diff(a: dict[str, float], b: dict[str, float], salt: str) -> dict:
    keys = sorted(set(a) & set(b))
    if not keys:
        return dict(n=0, mean=None, ci95=None)
    d = [a[k] - b[k] for k in keys]
    rng = random.Random(f"{SEED}-{salt}")
    means = sorted(mean(rng.choices(d, k=len(d))) for _ in range(BOOT))
    return dict(n=len(keys), mean=mean(d), ci95=[means[int(0.025 * BOOT)], means[int(0.975 * BOOT) - 1]],
                a_only=sum(x > 0 for x in d), b_only=sum(x < 0 for x in d))


def p95(values: list[float]) -> float | None:
    v = sorted(values)
    return v[min(len(v) - 1, math.ceil(0.95 * len(v)) - 1)] if v else None


def arm_summary(items: list[dict]) -> dict:
    sel = [t for t in items if t["arm"] in ("llm", "jev")]
    reasons = sorted({t["selection"]["reason"] for t in sel if t["selection"]["reason"]})
    toks = [total_tokens(t) for t in items]
    lat = [t["latency_s"] for t in items if t["latency_s"] is not None]
    recalls = [t["selection"]["support_recall"] for t in items if t["selection"]["support_recall"] is not None]
    asserted = [t for t in items if t["stratum"] != "none"]
    none = [t for t in items if t["stratum"] == "none"]
    return {
        "trials": len(items),
        "success": wilson([t["check"]["success"] for t in items]),
        "statuses": {s: sum(t["status"] == s for t in items) for s in contract.STATUSES},
        "applied": wilson([t["selection"]["applied"] == t["arm"] for t in sel]) if sel else None,
        "fallback_reasons": {r: sum(t["selection"]["reason"] == r for t in sel) for r in reasons},
        "support_recall": mean(recalls) if recalls else None,
        "selected_bytes_mean": mean(t["selection"]["bytes"] for t in items),
        "unsupported_answers": sum(1 for t in none if isinstance(t["answer"], dict) and t["answer"].get("value") is not None),
        "none_trials": len(none),
        "tokens_all_mean": mean(x["all"] for x in toks), "tokens_new_mean": mean(x["new"] for x in toks),
        "tokens_selection_mean": mean(x["selection"] for x in toks),
        "unreported_calls": sum(x["unreported"] for x in toks),
        "latency_mean_s": mean(lat) if lat else None, "latency_median_s": median(lat) if lat else None, "latency_p95_s": p95(lat),
        "asserted_trials": len(asserted),
    }


def compare(items: dict[str, list[dict]], provider: str) -> dict:
    ok = lambda t: float(t["check"]["success"])
    tok = lambda t: float(total_tokens(t)["all"])
    lat = lambda t: float(t["latency_s"] if t["latency_s"] is not None else 0.0)
    out = {}
    for a, b in (("jev", "code"), ("jev", "full"), ("jev", "llm"), ("llm", "code"), ("llm", "full")):
        out[f"{a}_minus_{b}"] = {
            "success": boot_diff(per_task(items[a], ok), per_task(items[b], ok), f"{provider}-{a}-{b}-s"),
            "tokens_all": boot_diff(per_task(items[a], tok), per_task(items[b], tok), f"{provider}-{a}-{b}-t"),
            "latency_s": boot_diff(per_task(items[a], lat), per_task(items[b], lat), f"{provider}-{a}-{b}-l"),
        }
    return out


def verdict(c: dict) -> dict:
    """사전 기준. 품질 허용폭은 0이다. 구간은 과제 단위 부트스트랩 95%."""
    q_code, q_full = c["jev_minus_code"]["success"], c["jev_minus_full"]["success"]
    t_llm, l_llm = c["jev_minus_llm"]["tokens_all"], c["jev_minus_llm"]["latency_s"]
    t_full, l_full = c["jev_minus_full"]["tokens_all"], c["jev_minus_full"]["latency_s"]
    if q_code["ci95"] is None:
        return dict(result="보류", reason="no data")
    gain = q_code["ci95"][0] > 0
    keep = q_full["ci95"][0] >= 0
    cheaper = t_full["ci95"][1] < 0 and t_llm["ci95"][1] < 0
    faster = l_full["ci95"][1] < 0 and l_llm["ci95"][1] < 0
    worse = q_code["ci95"][1] < 0 or q_full["ci95"][1] < 0
    parts = dict(quality_gain_over_code=gain, quality_kept_vs_full=keep, fewer_tokens_than_full_and_llm=cheaper, faster_than_full_and_llm=faster)
    result = "채택" if all(parts.values()) else "기각" if worse else "보류"
    return dict(result=result, **parts)


def sensitivity(items: dict[str, list[dict]], provider: str) -> dict:
    """수집 뒤에 더한 민감도 분석(판정에 쓰지 않는다). 답 앞뒤의 설명 글과 코드 펜스를 허용하는 추출로 성공을 다시 센다."""
    ok = lambda t: float(t["check"]["success_lenient"])
    out = {"success": {a: wilson([t["check"]["success_lenient"] for t in ts]) for a, ts in items.items()}, "paired": {}}
    for a, b in (("jev", "code"), ("jev", "full"), ("jev", "llm"), ("llm", "full")):
        out["paired"][f"{a}_minus_{b}"] = boot_diff(per_task(items[a], ok), per_task(items[b], ok), f"{provider}-{a}-{b}-lenient")
    return out


def slice_table(items: list[dict], key: str) -> dict:
    out = {}
    for v in sorted({t[key] for t in items}):
        part = [t for t in items if t[key] == v]
        out[v] = {a: wilson([t["check"]["success"] for t in part if t["arm"] == a]) for a in selection.ARMS}
    return out


def main() -> None:
    rows = [json.loads(s) for s in (PRIVATE / "trials.jsonl").read_text().splitlines()]
    lock = read(LOCK)
    out: dict = {"tau": lock["tau"], "budget_bytes": lock["budget_bytes"], "providers": {}}
    for provider in ("claude", "codex"):
        part = [t for t in rows if t["provider"] == provider]
        by_arm = {a: [t for t in part if t["arm"] == a] for a in selection.ARMS}
        cmp = compare(by_arm, provider)
        out["providers"][provider] = {
            "tasks": len({t["task_id"] for t in part}),
            "arms": {a: arm_summary(ts) for a, ts in by_arm.items()},
            "paired": cmp,
            "verdict": verdict(cmp),
            "sensitivity_lenient_extraction": sensitivity(by_arm, provider),
            "by_stratum": slice_table(part, "stratum"), "by_lang": slice_table(part, "lang"), "by_length": slice_table(part, "length"),
        }
    out["jev_calls_made"] = len({e["call_id"].split("@")[0] for t in rows for e in t["usage"] if e["role"] == "jev"})
    out["caller"] = CALLER
    out["evidence"] = "pipeline-only: stub caller, not a measurement of any selector" if CALLER == "stub" else "live calls through official CLI and TypeSafe HTTPS"
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print("analyzed", len(rows), "trials")


if __name__ == "__main__":
    main()
