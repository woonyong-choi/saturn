"""Recompute lazy and precompute judging when the judge asks every candidate.

Reads data/processed/turns.csv and results/summary.json. Writes
results/reanalysis-summary.json, results/tables/reanalysis-conditions.csv and two figures.
Not part of the preregistered analysis. The verdicts in 03-analyze.py stay as they are.
"""

import csv
import importlib.util
import json
import math
import os

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..")

spec = importlib.util.spec_from_file_location("analyze", os.path.join(HERE, "03-analyze.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

T_MAIN = base.T_MAIN
STARTS = base.STARTS
BASE_TOKENS = base.BASE_TOKENS
CAP_MAIN = base.CAP_MAIN
QUESTIONS_PER_CALL = base.QUESTIONS_PER_CALL
BOOT = base.BOOT
SEED = 119
BYTES_PER_TOKEN = 4
REQUEST_LIMIT_BYTES = 64_000
JUDGE_LATENCY_MS = 241
TOK_RATIO_LIMIT = 1.5
CUT_LIMIT = 50.0
# q_total, q_event, tok_total, tok_event, requests, pieces_event, events, q_wasted
METRICS = ("q_total", "q_event", "tok_total", "tok_event", "requests", "pieces_event", "events", "q_wasted")


def pieces(payload_tokens):
    """Requests needed for payload_tokens of questions. Each piece repeats the state (BASE_TOKENS)."""
    capacity = REQUEST_LIMIT_BYTES - BASE_TOKENS * BYTES_PER_TOKEN
    return max(1, math.ceil(payload_tokens * BYTES_PER_TOKEN / capacity))


def simulate(turns, t_limit, start, cap):
    """Return METRICS and the per-event (questions, pieces) list for one session. start=None is lazy."""
    packet = t_limit / 10
    level, prev = 0.0, None
    segment = []
    q_total = q_event = tok_total = tok_event = requests = pieces_event = events = 0
    q_since_event = 0
    event_rows = []
    for calls, chars, ctx, _ in turns:
        if ctx is not None:
            level += max(0, ctx - prev) if prev is not None else ctx
            prev = ctx
        item_tok = min(chars / calls / 4, cap) if calls else 0.0
        is_event = level >= t_limit
        judged = not is_event and start is not None and calls > 0 and level >= start * t_limit
        if judged:
            n = pieces(calls * item_tok)
            q = QUESTIONS_PER_CALL * calls
            q_total += q
            q_since_event += q
            tok_total += n * BASE_TOKENS + calls * item_tok
            requests += n
        if calls:
            segment.append((calls, item_tok, judged))
        if is_event:
            events += 1
            unjudged = sum(c for c, _, j in segment if not j)
            if unjudged:
                payload = sum(c * t for c, t, j in segment if not j)
                n = pieces(payload)
                q = QUESTIONS_PER_CALL * unjudged
                q_total += q
                q_event += q
                tok = n * BASE_TOKENS + payload
                tok_total += tok
                tok_event += tok
                requests += n
                pieces_event += n
                event_rows.append((q, n))
            else:
                event_rows.append((0, 0))
            segment = []
            q_since_event = 0
            level = packet
    return (q_total, q_event, tok_total, tok_event, requests, pieces_event, events, q_since_event), event_rows


def run(sessions, t_limit, start, cap=CAP_MAIN):
    mats, rows = [], []
    for s in sessions.values():
        m, e = simulate(s["turns"], t_limit, start, cap)
        mats.append(m)
        rows.extend(e)
    return np.array(mats, dtype=float), rows


def event_stats(rows):
    q = np.array([r[0] for r in rows], dtype=float)
    n = np.array([r[1] for r in rows], dtype=float)
    lat = n * JUDGE_LATENCY_MS
    return {
        "events": int(q.size),
        "pending_q_mean": base.r3(q.mean()),
        "pending_q_median": base.r3(np.median(q)),
        "pending_q_p95": base.r3(np.percentile(q, 95)),
        "pending_q_max": int(q.max()),
        "pieces_mean": base.r3(n.mean()),
        "pieces_p95": base.r3(np.percentile(n, 95)),
        "pieces_max": int(n.max()),
        "events_over_one_piece_pct": base.pct((n > 1).mean()),
        "latency_ms_mean": base.r3(lat.mean()),
        "latency_ms_median": base.r3(np.median(lat)),
        "latency_ms_p95": base.r3(np.percentile(lat, 95)),
        "latency_ms_max": base.r3(lat.max()),
    }


def main():
    sessions = base.load_sessions()
    results = os.path.join(ROOT, "results")
    with open(os.path.join(results, "summary.json"), encoding="utf-8") as f:
        previous = json.load(f)

    mats, ev_rows = {}, {}
    mats["lazy"], ev_rows["lazy"] = run(sessions, T_MAIN, None)
    for s in STARTS:
        mats[f"pre@{s}"], ev_rows[f"pre@{s}"] = run(sessions, T_MAIN, s)
    sums = {k: m.sum(axis=0) for k, m in mats.items()}
    rng = np.random.default_rng(SEED)
    boot = base.boot_sums(mats, rng)
    lazy, lazy_b = sums["lazy"], boot["lazy"]
    ix = {k: i for i, k in enumerate(METRICS)}

    def ratio_ci(num_b, den_b):
        r = num_b / den_b
        return [base.r3(v) for v in base.ci(r[np.isfinite(r)])]

    rows = []
    for s in STARTS:
        key = f"pre@{s}"
        pre, pre_b = sums[key], boot[key]
        cut_b = 1 - pre_b[:, ix["q_event"]] / lazy_b[:, ix["q_event"]]
        lat_cut_b = 1 - (pre_b[:, ix["pieces_event"]] / pre_b[:, ix["events"]]) / (
            lazy_b[:, ix["pieces_event"]] / lazy_b[:, ix["events"]])
        rows.append({
            "s": s,
            "q_total": int(pre[ix["q_total"]]),
            "q_event": int(pre[ix["q_event"]]),
            "q_wasted": int(pre[ix["q_wasted"]]),
            "tok_total": int(round(pre[ix["tok_total"]])),
            "requests": int(pre[ix["requests"]]),
            "q_ratio": base.r3(pre[0] / lazy[0]),
            "q_ratio_ci": ratio_ci(pre_b[:, 0], lazy_b[:, 0]),
            "tok_ratio": base.r3(pre[2] / lazy[2]),
            "tok_ratio_ci": ratio_ci(pre_b[:, 2], lazy_b[:, 2]),
            "request_ratio": base.r3(pre[4] / lazy[4]),
            "event_cut_pct": base.pct(1 - pre[1] / lazy[1]),
            "event_cut_pct_ci": [base.pct(v) for v in base.ci(cut_b[np.isfinite(cut_b)])],
            "latency_cut_pct": base.pct(1 - (pre[5] / pre[6]) / (lazy[5] / lazy[6])),
            "latency_cut_pct_ci": [base.pct(v) for v in base.ci(lat_cut_b[np.isfinite(lat_cut_b)])],
            "wasted_pct_of_pre_q": base.pct(pre[7] / pre[0]) if pre[0] else None,
            "wasted_pct_of_pre_q_ci": base.ci(
                np.where(pre_b[:, 0] > 0, pre_b[:, ix["q_wasted"]] / np.where(pre_b[:, 0] > 0, pre_b[:, 0], 1), 0)),
            "event_stats": event_stats(ev_rows[f"pre@{s}"]),
        })
        rows[-1]["wasted_pct_of_pre_q_ci"] = [base.pct(v) for v in rows[-1]["wasted_pct_of_pre_q_ci"]]
        rows[-1]["meets_criteria"] = (rows[-1]["tok_ratio_ci"][1] <= TOK_RATIO_LIMIT
                                      and rows[-1]["event_cut_pct_ci"][0] >= CUT_LIMIT)

    lazy_stats = event_stats(ev_rows["lazy"])
    prev_by_s = {r["s"]: r for r in previous["conditions"]}
    previous_rows = [{"s": s, "q_ratio": prev_by_s[s]["q_ratio"], "tok_ratio": prev_by_s[s]["tok_ratio"],
                      "pending_cut_pct": prev_by_s[s]["pending_cut_pct"],
                      "wasted_pct_of_pre_q": prev_by_s[s]["wasted_pct_of_pre_q"]} for s in STARTS]
    eligible = [r["s"] for r in rows if r["meets_criteria"]]
    summary = {
        "source_run_ids": previous["run_ids"],
        "params": {"T": T_MAIN, "base_tokens": BASE_TOKENS, "item_token_cap": CAP_MAIN,
                   "questions_per_call": QUESTIONS_PER_CALL, "bytes_per_token": BYTES_PER_TOKEN,
                   "request_limit_bytes": REQUEST_LIMIT_BYTES, "judge_latency_ms": JUDGE_LATENCY_MS,
                   "starts": list(STARTS), "bootstrap": BOOT, "seed": SEED,
                   "tok_ratio_limit": TOK_RATIO_LIMIT, "event_cut_limit_pct": CUT_LIMIT},
        "lazy": {
            "q_total": int(lazy[ix["q_total"]]),
            "tok_total": int(round(lazy[ix["tok_total"]])),
            "requests": int(lazy[ix["requests"]]),
            "events": int(lazy[ix["events"]]),
            "event_stats": lazy_stats,
        },
        "conditions": rows,
        "previous_top10": previous_rows,
        "previous_top10_lazy": {"q_total": previous["lazy"]["q_total"], "events": previous["lazy"]["events"],
                                "q_pending_per_event": base.r3(previous["lazy"]["q_pending"]
                                                               / previous["lazy"]["events"]),
                                "all_candidates_q_multiple": base.r3(lazy[ix["q_total"]] / previous["lazy"]["q_total"])},
        "criteria_met_starts": eligible,
    }
    with open(os.path.join(results, "reanalysis-summary.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)
        f.write("\n")

    base.write_csv(
        os.path.join(results, "tables", "reanalysis-conditions.csv"),
        ["condition", "q_total", "q_event", "q_wasted", "tok_total", "requests", "q_ratio", "q_ratio_lo",
         "q_ratio_hi", "tok_ratio", "tok_ratio_lo", "tok_ratio_hi", "event_cut_pct", "event_cut_lo",
         "event_cut_hi", "latency_cut_pct", "wasted_pct_of_pre_q"],
        [["lazy", summary["lazy"]["q_total"], summary["lazy"]["q_total"], 0, summary["lazy"]["tok_total"],
          summary["lazy"]["requests"], 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0]]
        + [[f"pre@{r['s']}", r["q_total"], r["q_event"], r["q_wasted"], r["tok_total"], r["requests"],
            r["q_ratio"], *r["q_ratio_ci"], r["tok_ratio"], *r["tok_ratio_ci"], r["event_cut_pct"],
            *r["event_cut_pct_ci"], r["latency_cut_pct"], r["wasted_pct_of_pre_q"]] for r in rows])

    n = len(sessions)
    figs = {
        "token-ratio-all-candidates": base.line_figure(
            "후보 전체 판단에서 미리 판단의 judge 입력 토큰 배수",
            f"n={n} session. 기준선은 1.5배", "lazy 대비 추정 입력 토큰(배)", "tok_ratio", TOK_RATIO_LIMIT, rows),
        "event-cut-all-candidates": base.line_figure(
            "후보 전체 판단에서 전환 시점 대기 질문의 감소율",
            f"n={n} session. 기준선은 50%", "전환 시점 대기 질문 감소율(%)", "event_cut_pct", CUT_LIMIT,
            [dict(r, event_cut_pct_ci=r["event_cut_pct_ci"]) for r in rows]),
    }
    for name, fig in figs.items():
        with open(os.path.join(results, "figures", f"{name}.vl.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(fig, f, ensure_ascii=False, indent=2)
            f.write("\n")


if __name__ == "__main__":
    main()
