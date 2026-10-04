"""trials.csv 에서 results/summary.json 과 results/tables/*.csv 를 만든다. 같은 입력이면 같은 바이트를 낸다."""
import csv
import json
from math import comb
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRIALS = ROOT / "data" / "processed" / "trials.csv"
RESULTS = ROOT / "results"


def cdf(k, n, p):
    return sum(comb(n, i) * p**i * (1 - p) ** (n - i) for i in range(k + 1))


def clopper_pearson(k, n, alpha=0.05):
    """정확 이항 신뢰구간(Clopper와 Pearson 1934). 이분법."""
    def solve(target, increasing):
        lo, hi = 0.0, 1.0
        for _ in range(60):
            mid = (lo + hi) / 2
            if (cdf_fn(mid) > target) == increasing:
                lo = mid
            else:
                hi = mid
        return (lo + hi) / 2
    if n == 0:
        return None
    if k == 0:
        low = 0.0
    else:
        cdf_fn = lambda p: 1 - cdf(k - 1, n, p)  # P(X >= k), p에 대해 증가
        low = solve(alpha / 2, False)
    if k == n:
        high = 1.0
    else:
        cdf_fn = lambda p: cdf(k, n, p)  # P(X <= k), p에 대해 감소
        high = solve(alpha / 2, True)
    return [round(low * 100, 1), round(high * 100, 1)]


def rate(rows):
    valid = [r for r in rows if r["outcome"] in ("accessed", "denied", "dialog")]
    k = sum(r["outcome"] == "accessed" for r in valid)
    n = len(valid)
    return {"accessed": k, "n": n, "percent": round(100 * k / n, 1) if n else None, "ci95": clopper_pearson(k, n),
            "denied": sum(r["outcome"] == "denied" for r in valid), "dialog": sum(r["outcome"] == "dialog" for r in valid)}


def extra(r):
    return json.loads(r["extra"])


def seconds(text):
    h, m, sec = text.split(":")
    return int(h) * 3600 + int(m) * 60 + float(sec)


def prompts_for(rows, source):
    """확인 창 표시 사건을 같은 호출 프로세스의 시험에 맞춘다. 시험 시작 추정 시각이 사건 시각보다 0.5초 넘게 늦지 않은 것 중 가장 늦은 시험."""
    layer = f"{source}-prompts"
    events = [r for r in rows if r["layer"] == layer and r["condition"] == "event"]
    trials = [r for r in rows if r["layer"] == layer and r["condition"] == "trial"]
    form = lambda r: "api-client" if r["condition"] and extra(r)["trial_condition"].endswith("api-client") else "direct"
    client_of = {"api-client": "Python.app", "direct": "security"}
    shown = {extra(t)["trial_ref"]: 0 for t in trials}
    approved = {extra(t)["trial_ref"]: 0 for t in trials}
    for e in events:
        x = extra(e)
        if x["event"] not in ("prompt_displayed",):
            continue
        at = seconds(x["event_utc"])
        cands = []
        for t in trials:
            tx = extra(t)
            if client_of[form(t)] != x["client"]:
                continue
            start = seconds(tx["end_utc"]) + 0.5 - tx["elapsed_s"]
            if start - 0.5 <= at:
                cands.append((start, tx["trial_ref"]))
        if cands:
            shown[max(cands)[1]] += 1
    approved_events = [e for e in events if extra(e)["event"] == "user_approved_always_allow"]
    by_trial = {}
    for t in trials:
        tx = extra(t)
        by_trial[tx["trial_condition"] + "#" + tx["trial_ref"]] = shown[tx["trial_ref"]]
    return {
        "prompts_displayed": sum(1 for e in events if extra(e)["event"] == "prompt_displayed"),
        "user_approved_always_allow": len(approved_events),
        "prompts_by_trial": by_trial,
        "trials_with_prompt": sum(1 for v in shown.values() if v),
    }


def main():
    rows = list(csv.DictReader(TRIALS.open(encoding="utf-8")))
    by = lambda layer: [r for r in rows if r["layer"] == layer]
    summary = {}

    codex = by("codex-sandbox")
    conds = sorted({r["condition"] for r in codex if "/" in r["condition"] and r["outcome"]})
    summary["h1"] = {c: rate([r for r in codex if r["condition"] == c]) for c in conds}
    summary["h1_exit_codes"] = {c: sorted({r["exit_code"] for r in codex if r["condition"] == c}) for c in conds}

    claude = by("claude-sandbox")
    cl_conds = sorted({r["condition"] for r in claude if r["outcome"]})
    summary["h2"] = {c: rate([r for r in claude if r["condition"] == c]) for c in cl_conds}
    summary["h2_exit_values"] = {c: sorted({str(r["bash_exit"]) for r in claude if r["condition"] == c}) for c in cl_conds}
    calls = [r for r in claude if r["condition"] == "calls"]
    summary["h2_calls_total"] = extra(calls[0])["total"] if calls else 0
    summary["h2_invalid_calls"] = sum(1 for r in claude if r["condition"] not in ("setup", "calls", "cleanup") and "call" in extra(r) and not r["outcome"])
    summary["h2_disable_sandbox_used"] = sum(1 for r in claude if extra(r).get("used_disable_sandbox"))
    summary["h2_login"] = {r["condition"]: extra(r)["logged_in"] for r in claude if r["condition"].startswith("login-")}
    summary["h2_all_calls_succeeded"] = all(extra(r).get("final", {}) and not extra(r)["final"].get("is_error") for r in claude if "call" in extra(r))

    acl = by("keychain-acl")
    summary["h3"] = {r["condition"]: {"outcome": r["outcome"], "exit_code": r["exit_code"]} for r in acl if r["outcome"]}

    env = by("env")
    summary["h4"] = {r["condition"]: extra(r)["passed"] for r in env}

    hook = by("hook")
    summary["h5_commits_ahead"] = extra(hook[0])["ahead_of_main"] if hook else None

    summary["prompts"] = {src: prompts_for(rows, src) for src in ("keychain-acl", "codex-sandbox", "claude-sandbox")}

    cleanup = {}
    for layer in ("keychain-acl", "codex-sandbox", "claude-sandbox"):
        for r in by(layer):
            if r["condition"] == "cleanup":
                cleanup[layer] = not any(extra(r)["remaining"].values())
    summary["cleanup_all_absent"] = cleanup

    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tables = RESULTS / "tables"
    tables.mkdir(exist_ok=True)
    for name, data in (("h1", summary["h1"]), ("h2", summary["h2"])):
        with (tables / f"{name}.csv").open("w", encoding="utf-8", newline="") as f:
            w = csv.writer(f, lineterminator="\n")
            w.writerow(["condition", "accessed", "n", "percent", "ci_low", "ci_high", "denied", "dialog"])
            for c, v in data.items():
                ci = v["ci95"] or ["", ""]
                w.writerow([c, v["accessed"], v["n"], v["percent"], ci[0], ci[1], v["denied"], v["dialog"]])


main()
