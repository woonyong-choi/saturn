"""Simulate lazy and precompute judging on processed turns and write results."""

import csv
import json
import os
from collections import OrderedDict

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..")
T_MAIN = 100_000
N_TOP = 10
BASE_TOKENS = 500
CAP_MAIN = 1_000
QUESTIONS_PER_CALL = 2
STARTS = (0.0, 0.25, 0.5, 0.6, 0.7, 0.8, 0.9)
BOOT = 10_000
SEED = 119
ALPHA = 0.05
MIN_EVENTS = 30
METRICS = ("q_total", "q_pending", "tok_total", "tok_pending", "events", "q_wasted")


def load_sessions():
    sessions = OrderedDict()
    with open(os.path.join(ROOT, "data", "processed", "turns.csv"), encoding="utf-8") as f:
        for r in csv.DictReader(f):
            s = sessions.setdefault(r["session_id"], {"provider": r["provider"], "run_id": r["run_id"], "turns": []})
            s["turns"].append((
                int(r["tool_calls"]),
                int(r["tool_io_chars"]),
                None if r["context_tokens"] == "" else int(r["context_tokens"]),
                r["compacted"] == "1",
            ))
    return sessions


def pending_calls(segment, mode):
    """Unjudged calls among the top N candidates at an event.

    segment holds (calls, item_tokens, judged) per turn since the last event.
    upper: top N take unjudged calls first. uniform: top N are a uniform draw.
    recency: top N are the most recent calls.
    Returns (count, mean item tokens of those calls).
    """
    total = sum(c for c, _, _ in segment)
    unjudged = sum(c for c, _, j in segment if not j)
    if not total or not unjudged:
        return 0.0, 0.0
    mean_tok = sum(c * t for c, t, j in segment if not j) / unjudged
    k = min(N_TOP, total)
    if mode == "upper":
        return float(min(k, unjudged)), mean_tok
    if mode == "uniform":
        return k * unjudged / total, mean_tok
    left, count, tok = k, 0.0, 0.0
    for c, t, j in reversed(segment):
        take = min(left, c)
        if not j:
            count += take
            tok += take * t
        left -= take
        if not left:
            break
    return count, (tok / count if count else 0.0)


def simulate(turns, t_limit, start, cap, mode="uniform"):
    """Return METRICS for one session. start=None is the lazy condition."""
    packet = t_limit / 10
    level, prev = 0.0, None
    segment = []
    q_total = q_pending = tok_total = tok_pending = events = q_since_event = 0.0
    for calls, chars, ctx, _ in turns:
        if ctx is not None:
            level += max(0, ctx - prev) if prev is not None else ctx
            prev = ctx
        item_tok = min(chars / calls / 4, cap) if calls else 0.0
        is_event = level >= t_limit
        judged = not is_event and start is not None and calls > 0 and level >= start * t_limit
        if judged:
            q = QUESTIONS_PER_CALL * calls
            q_total += q
            tok_total += BASE_TOKENS + calls * item_tok
            q_since_event += q
        if calls:
            segment.append((calls, item_tok, judged))
        if is_event:
            events += 1
            k, mean_tok = pending_calls(segment, mode)
            if k:
                q = QUESTIONS_PER_CALL * k
                tok = BASE_TOKENS + k * mean_tok
                q_total += q
                q_pending += q
                tok_total += tok
                tok_pending += tok
            segment = []
            q_since_event = 0.0
            level = packet
    return (q_total, q_pending, tok_total, tok_pending, events, q_since_event)


def matrix(sessions, t_limit, start, cap, mode="uniform"):
    return np.array([simulate(s["turns"], t_limit, start, cap, mode) for s in sessions.values()], dtype=float)


def boot_sums(mats, rng):
    """Bootstrap column sums for each matrix with shared session resamples."""
    n = next(iter(mats.values())).shape[0]
    out = {k: [] for k in mats}
    done = 0
    while done < BOOT:
        b = min(500, BOOT - done)
        weights = rng.multinomial(n, np.full(n, 1.0 / n), size=b).astype(float)
        for k, m in mats.items():
            out[k].append(weights @ m)
        done += b
    return {k: np.vstack(v) for k, v in out.items()}


def ci(values):
    lo, hi = np.percentile(values, [2.5, 97.5])
    return [float(lo), float(hi)]


def r3(x):
    return round(float(x), 3)


def pct(x):
    return round(100 * float(x), 1)


def describe(values):
    v = np.asarray(values, dtype=float)
    if v.size == 0:
        return None
    return {
        "n": int(v.size),
        "mean": r3(v.mean()),
        "sd": r3(v.std(ddof=1)) if v.size > 1 else 0.0,
        "median": r3(np.median(v)),
        "p5": r3(np.percentile(v, 5)),
        "p95": r3(np.percentile(v, 95)),
    }


def wilson(k, n, z=1.959964):
    if not n:
        return None
    p = k / n
    centre = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * np.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return [pct(centre - half), pct(centre + half)]


def holm(pvalues):
    order = sorted(pvalues, key=pvalues.get)
    passed, m, ok = {}, len(order), True
    for i, key in enumerate(order):
        ok = ok and pvalues[key] <= ALPHA / (m - i)
        passed[key] = ok
    return passed


def point_ratios(sessions, t_limit, cap, mode="uniform"):
    lazy = matrix(sessions, t_limit, None, cap, mode).sum(axis=0)
    rows = []
    for s in STARTS:
        pre = matrix(sessions, t_limit, s, cap, mode).sum(axis=0)
        rows.append({
            "s": s,
            "q_ratio": r3(pre[0] / lazy[0]) if lazy[0] else None,
            "tok_ratio": r3(pre[2] / lazy[2]) if lazy[2] else None,
            "pending_cut_pct": pct(1 - pre[1] / lazy[1]) if lazy[1] else None,
            "wasted_pct_of_pre_q": pct(pre[5] / pre[0]) if pre[0] else None,
        })
    return {"events": int(lazy[4]), "rows": rows}


def write_csv(path, header, rows):
    with open(path, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(header)
        w.writerows(rows)


def line_figure(title, subtitle, y_title, field, threshold, rows):
    values = [{"s": r["s"], "value": r[field], "lo": r[field + "_ci"][0], "hi": r[field + "_ci"][1]} for r in rows]
    x = {"field": "s", "type": "quantitative", "title": "미리 판단 시작 비율 s(T 대비)"}
    return {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": title, "subtitle": subtitle},
        "width": 420,
        "height": 220,
        "layer": [
            {"data": {"values": values}, "mark": {"type": "line", "point": True},
             "encoding": {"x": x, "y": {"field": "value", "type": "quantitative", "title": y_title,
                                        "scale": {"zero": True}}}},
            {"data": {"values": values}, "mark": "errorbar",
             "encoding": {"x": x, "y": {"field": "lo", "type": "quantitative", "title": y_title},
                          "y2": {"field": "hi"}}},
            {"data": {"values": [{"threshold": threshold}]}, "mark": "rule",
             "encoding": {"y": {"field": "threshold", "type": "quantitative"}}},
        ],
    }


def main():
    sessions = load_sessions()
    results = os.path.join(ROOT, "results")
    os.makedirs(os.path.join(results, "tables"), exist_ok=True)
    os.makedirs(os.path.join(results, "figures"), exist_ok=True)
    run_ids = sorted({s["run_id"] for s in sessions.values()})
    meta_files = [os.path.join(ROOT, "data", "raw", f"agent-logs-{rid}.meta.json") for rid in run_ids]
    flow = {}
    for path in meta_files:
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                meta = json.load(f)
            flow = {p: meta[p] for p in ("claude", "codex") if p in meta}
    if flow:
        # 01-collect counts Claude subagent files before "files" and Codex subagent files inside it.
        claude, codex = flow.get("claude", {}), flow.get("codex", {})
        keys = ("excluded_subagent_files", "excluded_no_turns", "excluded_no_tokens", "excluded_duplicate",
                "failed_files", "bad_lines", "sessions")
        flow["total"] = {k: claude.get(k, 0) + codex.get(k, 0) for k in keys}
        flow["total"]["files_read"] = claude.get("files", 0) + claude.get("excluded_subagent_files", 0) + codex.get(
            "files", 0)

    mats = {"lazy": matrix(sessions, T_MAIN, None, CAP_MAIN)}
    for s in STARTS:
        mats[f"pre@{s}"] = matrix(sessions, T_MAIN, s, CAP_MAIN)
    sums = {k: m.sum(axis=0) for k, m in mats.items()}
    rng = np.random.default_rng(SEED)
    boot = boot_sums(mats, rng)

    lazy, lazy_b = sums["lazy"], boot["lazy"]
    cond_rows = []
    for s in STARTS:
        key = f"pre@{s}"
        pre, pre_b = sums[key], boot[key]
        with np.errstate(divide="ignore", invalid="ignore"):
            q_ratio_b = pre_b[:, 0] / lazy_b[:, 0]
            tok_ratio_b = pre_b[:, 2] / lazy_b[:, 2]
            cut_b = 1 - pre_b[:, 1] / lazy_b[:, 1]
            tokcut_b = 1 - pre_b[:, 3] / lazy_b[:, 3]
        cond_rows.append({
            "s": s,
            "q_total": int(pre[0]),
            "q_pending": int(pre[1]),
            "tok_total": int(round(pre[2])),
            "tok_pending": int(round(pre[3])),
            "q_wasted": int(pre[5]),
            "q_ratio": r3(pre[0] / lazy[0]),
            "q_ratio_ci": [r3(v) for v in ci(q_ratio_b[np.isfinite(q_ratio_b)])],
            "tok_ratio": r3(pre[2] / lazy[2]),
            "tok_ratio_ci": [r3(v) for v in ci(tok_ratio_b[np.isfinite(tok_ratio_b)])],
            "pending_cut_pct": pct(1 - pre[1] / lazy[1]),
            "pending_cut_pct_ci": [pct(v) for v in ci(cut_b[np.isfinite(cut_b)])],
            "pending_tok_cut_pct": pct(1 - pre[3] / lazy[3]),
            "pending_tok_cut_pct_ci": [pct(v) for v in ci(tokcut_b[np.isfinite(tokcut_b)])],
            "wasted_pct_of_pre_q": pct(pre[5] / pre[0]) if pre[0] else None,
            "_boot": {"q_ratio": q_ratio_b, "tok_ratio": tok_ratio_b, "cut": cut_b},
        })
    by_s = {r["s"]: r for r in cond_rows}

    p_values = {
        "H1": float(np.mean(~(by_s[0.0]["_boot"]["q_ratio"] >= 3))),
        "H2": float(np.mean(~(by_s[0.5]["_boot"]["cut"] >= 0.5))),
        "H3": float(np.mean(~(by_s[0.5]["_boot"]["tok_ratio"] <= 1.5))),
    }
    holm_pass = holm(p_values)
    criteria = {
        "H1": by_s[0.0]["q_ratio_ci"][0] >= 3,
        "H2": by_s[0.5]["pending_cut_pct_ci"][0] >= 50,
        "H3": by_s[0.5]["tok_ratio_ci"][1] <= 1.5,
    }
    enough = lazy[4] >= MIN_EVENTS
    verdict = {}
    for h in ("H1", "H2", "H3"):
        if not enough:
            verdict[h] = "보류"
        else:
            verdict[h] = "채택" if criteria[h] and holm_pass[h] else "기각"
    eligible = [r["s"] for r in cond_rows if r["tok_ratio_ci"][1] <= 1.5]
    s_star = min(eligible) if eligible else None
    star = by_s.get(s_star)
    if not enough:
        decision = "보류"
    elif verdict["H2"] == "채택" and verdict["H3"] == "채택":
        decision = f"pre@{s_star}"
    elif verdict["H2"] == "기각":
        decision = "lazy"
    elif star is not None and star["pending_cut_pct_ci"][0] >= 50:
        decision = f"pre@{s_star}"
    else:
        decision = "lazy"

    per_session = {k: m for k, m in mats.items()}
    providers = [s["provider"] for s in sessions.values()]
    desc = {}
    for prov in ("all", "claude", "codex"):
        idx = [i for i, p in enumerate(providers) if prov == "all" or p == prov]
        ss = [list(sessions.values())[i] for i in idx]
        if not ss:
            continue
        turns_n = [len(s["turns"]) for s in ss]
        calls_turn = [t[0] for s in ss for t in s["turns"]]
        max_ctx = [max(t[2] for t in s["turns"] if t[2] is not None) for s in ss]
        reached = int((per_session["lazy"][idx, 4] > 0).sum())
        compacted = sum(1 for s in ss if any(t[3] for t in s["turns"]))
        desc[prov] = {
            "sessions": len(ss),
            "turns": int(sum(turns_n)),
            "tool_calls": int(sum(calls_turn)),
            "turns_per_session": describe(turns_n),
            "tool_calls_per_turn": describe(calls_turn),
            "max_context_tokens": describe(max_ctx),
            "sessions_reaching_event": reached,
            "sessions_reaching_event_pct": pct(reached / len(ss)),
            "sessions_reaching_event_pct_ci": wilson(reached, len(ss)),
            "sessions_with_provider_compaction": compacted,
            "sessions_with_provider_compaction_pct": pct(compacted / len(ss)),
            "sessions_with_provider_compaction_pct_ci": wilson(compacted, len(ss)),
            "events": int(per_session["lazy"][idx, 4].sum()),
        }

    explore = {
        "t_60000": point_ratios(sessions, 60_000, CAP_MAIN),
        "t_150000": point_ratios(sessions, 150_000, CAP_MAIN),
        "cap_75": point_ratios(sessions, T_MAIN, 75),
        "cap_4000": point_ratios(sessions, T_MAIN, 4_000),
        "pending_upper": point_ratios(sessions, T_MAIN, CAP_MAIN, "upper"),
        "pending_recency": point_ratios(sessions, T_MAIN, CAP_MAIN, "recency"),
    }
    for prov in ("claude", "codex"):
        sub = OrderedDict((k, v) for k, v in sessions.items() if v["provider"] == prov)
        if sub:
            explore[f"provider_{prov}"] = point_ratios(sub, T_MAIN, CAP_MAIN)

    public_rows = [{k: v for k, v in r.items() if k != "_boot"} for r in cond_rows]
    summary = {
        "run_ids": run_ids,
        "params": {"T": T_MAIN, "N": N_TOP, "base_tokens": BASE_TOKENS, "item_token_cap": CAP_MAIN,
                   "questions_per_call": QUESTIONS_PER_CALL, "starts": list(STARTS), "bootstrap": BOOT,
                   "seed": SEED, "alpha": ALPHA, "min_events": MIN_EVENTS},
        "flow": flow,
        "sample": desc,
        "lazy": {"q_total": int(lazy[0]), "q_pending": int(lazy[1]), "tok_total": int(round(lazy[2])),
                 "tok_pending": int(round(lazy[3])), "events": int(lazy[4])},
        "conditions": public_rows,
        "hypotheses": {
            h: {"p_one_sided": round(p_values[h], 4), "holm_pass": holm_pass[h], "criterion_met": criteria[h],
                "verdict": verdict[h]} for h in ("H1", "H2", "H3")
        },
        "decision": {"choice": decision, "s_star": s_star, "eligible_starts": eligible},
        "exploratory": explore,
    }
    with open(os.path.join(results, "summary.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)
        f.write("\n")

    write_csv(os.path.join(results, "tables", "conditions.csv"),
              ["condition", "q_total", "q_pending", "tok_total", "tok_pending", "q_ratio", "q_ratio_lo", "q_ratio_hi",
               "tok_ratio", "tok_ratio_lo", "tok_ratio_hi", "pending_cut_pct", "pending_cut_lo", "pending_cut_hi"],
              [["lazy", int(lazy[0]), int(lazy[1]), int(round(lazy[2])), int(round(lazy[3])),
                1, 1, 1, 1, 1, 1, 0, 0, 0]] +
              [[f"pre@{r['s']}", r["q_total"], r["q_pending"], r["tok_total"], r["tok_pending"], r["q_ratio"],
                *r["q_ratio_ci"], r["tok_ratio"], *r["tok_ratio_ci"], r["pending_cut_pct"],
                *r["pending_cut_pct_ci"]] for r in public_rows])
    write_csv(os.path.join(results, "tables", "sample.csv"),
              ["provider", "sessions", "turns", "tool_calls", "turns_median", "turns_p95", "max_ctx_median",
               "max_ctx_p95", "sessions_reaching_event_pct", "sessions_with_provider_compaction_pct", "events"],
              [[p, d["sessions"], d["turns"], d["tool_calls"], d["turns_per_session"]["median"],
                d["turns_per_session"]["p95"], d["max_context_tokens"]["median"], d["max_context_tokens"]["p95"],
                d["sessions_reaching_event_pct"], d["sessions_with_provider_compaction_pct"], d["events"]]
               for p, d in desc.items()])

    n = len(sessions)
    figs = {
        "token-ratio-by-start": line_figure(
            "미리 판단의 judge 입력 토큰 배수", f"n={n} session. 기준선은 채택 기준 1.5배",
            "lazy 대비 추정 입력 토큰(배)", "tok_ratio", 1.5, public_rows),
        "pending-cut-by-start": line_figure(
            "정리 시점에 남는 질문의 감소율", f"n={n} session. 기준선은 채택 기준 50%",
            "남는 질문 감소율(%)", "pending_cut_pct", 50, public_rows),
    }
    for name, spec in figs.items():
        with open(os.path.join(results, "figures", f"{name}.vl.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(spec, f, ensure_ascii=False, indent=2)
            f.write("\n")


if __name__ == "__main__":
    main()
