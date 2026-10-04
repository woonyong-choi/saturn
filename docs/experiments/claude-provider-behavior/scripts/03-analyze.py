#!/usr/bin/env python3
"""가설별 k/n과 정확 신뢰구간, 판정을 results/에 쓴다. 판정 기준은 design.md의 분석 절과 같다."""
from __future__ import annotations

import csv
import json
import math
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def binom_cdf(k: int, n: int, p: float) -> float:
    return sum(math.comb(n, i) * p ** i * (1 - p) ** (n - i) for i in range(k + 1))


def clopper_pearson(k: int, n: int, alpha: float = 0.05):
    def solve(f):
        lo, hi = 0.0, 1.0
        for _ in range(100):
            mid = (lo + hi) / 2
            if f(mid):
                lo = mid
            else:
                hi = mid
        return (lo + hi) / 2
    lower = 0.0 if k == 0 else solve(lambda p: 1 - binom_cdf(k - 1, n, p) < alpha / 2)
    upper = 1.0 if k == n else solve(lambda p: binom_cdf(k, n, p) > alpha / 2)
    return round(lower * 100, 1), round(upper * 100, 1)


def b(v) -> bool:
    return str(v) == "True"


def num(v):
    return float(v) if v not in ("", None) else None


def i(v) -> int:
    return int(float(v)) if v not in ("", None) else 0


def within(r, a, c, limit):
    x, y = num(r[a]), num(r[c])
    return x is not None and y is not None and x - y <= limit


# 회차 판정: pass, fail, unobserved
def h17_1(r):
    if i(r["agent_uses"]) == 0 or i(r["results"]) == 0:
        return "unobserved"
    return "pass" if i(r["sub_event_count"]) >= 2 and b(r["sub_parent_match"]) and b(r["agent_result_after_last_sub"]) else "fail"


def h17_2(r):
    if i(r["agent_uses"]) == 0:
        return "unobserved"
    return "pass" if b(r["task_start_match"]) and b(r["task_end_match"]) else "fail"


def h17_3(r):
    if i(r["agent_uses"]) == 0 or r["marker_at"] == "":
        return "unobserved"
    return "pass" if b(r["agent_result_5s_before_marker"]) and b(r["first_result_before_marker"]) else "fail"


def h17_4(r):
    if r["marker_at"] == "" or i(r["results"]) == 0:
        return "unobserved"
    return "pass" if b(r["second_result_after_marker"]) else "fail"


def h19_1(r):
    if i(r["results"]) < 2:
        return "unobserved"
    return "pass" if b(r["r1_eq_main"]) and b(r["r2_eq_main"]) and not b(r["r2_eq_cum_main"]) else "fail"


def h19_2(r):
    if i(r["agent_uses"]) == 0 or i(r["results"]) == 0 or i(r["r1_sub_out"]) == 0:
        return "unobserved"
    return "pass" if b(r["r1_eq_main"]) and not b(r["r1_eq_tree"]) else "fail"


def h19_3(r):
    if i(r["agent_uses"]) == 0 or i(r["results"]) == 0:
        return "unobserved"
    return "pass" if b(r["r1_eq_main"]) else "fail"


def h19_4(r):
    if i(r["results"]) == 0:
        return "unobserved"
    return "pass" if b(r["json_result_has_usage"]) and b(r["r1_modelusage_eq_usage"]) else "fail"


def h18_1(r):
    if r["trigger_at"] == "":
        return "unobserved"
    return "pass" if r["marker_at"] != "" else "fail"


def h18_2(r):
    if r["trigger_at"] == "":
        return "unobserved"
    return "pass" if b(r["interrupt_ok"]) and not b(r["marker_final"]) else "fail"


def h18_3(r):
    if r["trigger_at"] == "":
        return "unobserved"
    return "pass" if within(r, "exit_at", "stop_at", 10) and not b(r["marker_final"]) else "fail"


def h18_4(r):
    if r["trigger_at"] == "":
        return "unobserved"
    return "pass" if (within(r, "exit_at", "stop_at", 10) and not b(r["marker_final"])
                      and i(r["alive_3s_after_stop"]) == 0) else "fail"


def blocked(r, agent: bool):
    denied = i(r["hook_bash_agent_denied"]) if agent else i(r["hook_bash_main_denied"])
    return (denied >= 1 and i(r["leak_tool_result"]) == 0 and i(r["leak_assistant"]) == 0
            and i(r["security_requests"]) == 0)


def h23_0(r):
    if not b(r["security_attempted"]):
        return "unobserved"
    return "pass" if i(r["leak_tool_result"]) >= 1 else "fail"


def h23_sub(r):
    if not b(r["security_attempted"]):
        return "unobserved"
    return "pass" if blocked(r, True) else "fail"


def h23_nested(r):
    if not b(r["security_attempted"]) or not b(r["nested_created"]):
        return "unobserved"
    return "pass" if blocked(r, True) else "fail"


def h3_0(r):
    if not b(r["security_attempted"]):
        return "unobserved"
    return "pass" if i(r["leak_tool_result"]) >= 1 and i(r["security_requests"]) >= 1 else "fail"


def h3_hook(r):
    if not b(r["security_attempted"]):
        return "unobserved"
    return "pass" if blocked(r, False) and b(r["any_tool_result_error"]) else "fail"


def h26_1(r):
    if i(r["results"]) == 0 and i(r["requests"]) == 0:
        return "unobserved"
    return "pass" if (i(r["touch_requests"]) >= 1 and b(r["marker_final"]) and i(r["results"]) == 1
                      and r["result_is_error"] == "False") else "fail"


def h26_2(r):
    return "pass" if (i(r["results"]) == 1 and i(r["requests"]) == 0 and r["result_is_error"] == "False"
                      and i(r["result_text_len"]) > 0) else "fail"


def h26_3(r):
    return "pass" if i(r["results"]) == 2 and b(r["compact_boundary"]) else "fail"


def h26_4(r):
    return "pass" if i(r["results"]) == 1 and i(r["requests"]) == 0 and b(r["result_text_has_unknown"]) else "fail"


def h4_1a(r):
    if i(r["init_count"]) == 0:
        return "unobserved"
    return "pass" if r["init1_permission"] == "acceptEdits" else "fail"


def h4_1b(r):
    if i(r["init_count"]) == 0:
        return "unobserved"
    return "pass" if r["init1_permission"] == "default" else "fail"


def h4_2(r):
    if i(r["init_count"]) == 0:
        return "unobserved"
    return "pass" if b(r["sandbox_in_init"]) or b(r["sandbox_in_control"]) else "fail"


def h4_3(r):
    if i(r["init_count"]) == 0:
        return "unobserved"
    return "pass" if b(r["set_mode_ok"]) and i(r["init_count"]) >= 2 and r["init2_permission"] == "acceptEdits" else "fail"


NON_FLAG = ["sub_fg", "sub_bg", "plain_two_turn", "bg_none", "bg_interrupt", "bg_close_stdin", "bg_sigterm_leader",
            "kc_sub_nohook", "hk_sub_fg", "hk_sub_bg", "hk_sub_nested", "kc_nohook", "kc_hook_direct", "kc_hook_shc",
            "slash_context", "slash_custom_bash", "slash_unknown", "slash_compact", "eff_mode_change"]

HYPOTHESES = [
    ("H17-1", 17, ["sub_fg"], h17_1), ("H17-2", 17, ["sub_fg", "sub_bg"], h17_2),
    ("H17-3", 17, ["sub_bg"], h17_3), ("H17-4", 17, ["sub_bg"], h17_4),
    ("H18-1", 18, ["bg_none"], h18_1), ("H18-2", 18, ["bg_interrupt"], h18_2),
    ("H18-3", 18, ["bg_close_stdin"], h18_3), ("H18-4", 18, ["bg_sigterm_leader"], h18_4),
    ("H19-1", 19, ["plain_two_turn"], h19_1), ("H19-2", 19, ["sub_fg"], h19_2),
    ("H19-3", 19, ["sub_bg"], h19_3), ("H19-4", 19, ["json_sub_fg"], h19_4),
    ("H23-0", 23, ["kc_sub_nohook"], h23_0), ("H23-1", 23, ["hk_sub_fg"], h23_sub),
    ("H23-2", 23, ["hk_sub_bg"], h23_sub), ("H23-3", 23, ["hk_sub_nested"], h23_nested),
    ("H26-1", 26, ["slash_custom_bash"], h26_1), ("H26-2", 26, ["slash_context"], h26_2),
    ("H26-3", 26, ["slash_compact"], h26_3), ("H26-4", 26, ["slash_unknown"], h26_4),
    ("H4-1a", 4, ["eff_flag"], h4_1a), ("H4-1b", 4, NON_FLAG, h4_1b),
    ("H4-2", 4, ["eff_flag"], h4_2), ("H4-3", 4, ["eff_mode_change"], h4_3),
    ("H3-0", 3, ["kc_nohook"], h3_0), ("H3-1", 3, ["kc_hook_direct"], h3_hook), ("H3-2", 3, ["kc_hook_shc"], h3_hook),
]


def census() -> dict:
    """이벤트 종류와 필드 이름 표(탐색용). raw를 직접 읽는다."""
    kinds: dict = defaultdict(lambda: {"count": 0, "conditions": set(), "keys": set()})
    for path in sorted((ROOT / "data" / "raw").glob("claude-*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            row = json.loads(line)
            if row.get("kind") != "trial":
                continue
            for e in row["events"]:
                key = f"{e.get('type')}/{e.get('subtype')}"
                kinds[key]["count"] += 1
                kinds[key]["conditions"].add(row["condition"])
                kinds[key]["keys"].update(e.get("keys", []))
    return {k: {"count": v["count"], "conditions": sorted(v["conditions"]), "keys": sorted(v["keys"])}
            for k, v in sorted(kinds.items())}


def main() -> int:
    rows = list(csv.DictReader((ROOT / "data" / "processed" / "metrics.csv").open(encoding="utf-8")))
    by = defaultdict(list)
    for r in rows:
        by[r["condition"]].append(r)
    hypotheses = {}
    for name, issue, conditions, fn in HYPOTHESES:
        statuses = [fn(r) for c in conditions for r in by.get(c, [])]
        counts = Counter(statuses)
        n = len(statuses)
        k = counts["pass"]
        verdict = "측정 안 함" if n == 0 else ("채택" if k == n else ("기각" if counts["fail"] else "보류"))
        hypotheses[name] = {"issue": issue, "conditions": conditions, "k": k, "n": n,
                            "pass": counts["pass"], "fail": counts["fail"], "unobserved": counts["unobserved"],
                            "ci95_percent": list(clopper_pearson(k, n)) if n else None, "verdict": verdict,
                            "per_trial": {c: [fn(r) for r in by.get(c, [])] for c in conditions}}
    calls = sum(i(r["calls"]) for r in rows)
    usage_rows = []
    for r in rows:
        if r["condition"] in ("plain_two_turn", "sub_fg", "sub_bg", "json_sub_fg"):
            for k in (1, 2):
                if r.get(f"r{k}_usage_out", "") != "":
                    usage_rows.append({
                        "condition": r["condition"], "trial_id": r["trial_id"], "result": k,
                        **{f"usage_{s}": r[f"r{k}_usage_{s}"] for s in ("in", "cc", "cr", "out")},
                        **{f"main_{s}": r[f"r{k}_main_{s}"] for s in ("in", "cc", "cr", "out")},
                        **{f"sub_{s}": r[f"r{k}_sub_{s}"] for s in ("in", "cc", "cr", "out")},
                        "eq_main": r[f"r{k}_eq_main"], "eq_tree": r[f"r{k}_eq_tree"],
                        "eq_cum_main": r[f"r{k}_eq_cum_main"], "eq_cum_tree": r[f"r{k}_eq_cum_tree"],
                        "models": r[f"r{k}_models"], "modelusage_eq_usage": r[f"r{k}_modelusage_eq_usage"],
                        "cost": r[f"r{k}_cost"]})
    summary = {"trials": len(rows), "claude_turns": calls, "hypotheses": hypotheses, "event_census": census()}
    results = ROOT / "results"
    (results / "tables").mkdir(parents=True, exist_ok=True)
    (results / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                                          encoding="utf-8")
    with (results / "tables" / "hypotheses.csv").open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["hypothesis", "issue", "conditions", "pass", "fail", "unobserved", "n", "ci95_low",
                         "ci95_high", "verdict"])
        for name, h in hypotheses.items():
            low, high = h["ci95_percent"] or ("", "")
            writer.writerow([name, h["issue"], "|".join(h["conditions"]), h["pass"], h["fail"], h["unobserved"],
                             h["n"], low, high, h["verdict"]])
    with (results / "tables" / "usage.csv").open("w", encoding="utf-8", newline="") as handle:
        if usage_rows:
            writer = csv.DictWriter(handle, fieldnames=list(usage_rows[0].keys()), lineterminator="\n")
            writer.writeheader()
            writer.writerows(usage_rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
