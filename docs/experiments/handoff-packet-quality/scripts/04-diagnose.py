"""근거 항목이 패킷에서 빠진 단계를 세고, 기준값과 채우기 규칙을 바꿨을 때의 근거 포함률을 다시 계산한다.

사용: python3 scripts/04-diagnose.py (실험 폴더에서 실행)
입력은 data/raw/의 시나리오와 패킷(judge 판단 확률 포함), data/processed/trials.csv다.
새 수집은 없다. 패킷 채우기 규칙은 saturn-core의 packet 예제와 sessions/packet.rs의 규칙을 파이썬으로 옮긴 것이고,
실제 두 조건의 포함 목록과 일치하는지 먼저 확인한 값을 summary에 남긴다.
출력: results/diagnosis-summary.json, results/tables/diagnosis-*.csv
사전 등록 판정에는 쓰지 않고 설계 판단 근거로만 쓴다.
"""
from __future__ import annotations

import csv
import json
import math
import re
from collections import defaultdict
from pathlib import Path

import numpy as np

SEED = 209
BOOTSTRAP_REPS = 10000
Z95 = 1.959963984540054
EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"
PROCESSED = EXPERIMENT / "data" / "processed"
RESULTS = EXPERIMENT / "results"
BUDGET_TOKENS = 8000
CHARS_PER_TOKEN = 4
ITEM_SHARE_PERCENT = 30
DIGEST_HEAD_CHARS = 300
ITEM_SEPARATOR = "\n\n"
COMPETING_TITLE = "Earlier records"
PROVIDER_DOCS = ("AGENTS.md", "CLAUDE.md")
ACTUAL_THRESHOLD = 0.5
THRESHOLDS = [0.5, 0.4, 0.3, 0.25, 0.2, 0.1, 0.0]
QTYPES = ["extract", "multi-session", "temporal", "update"]
KINDS = ["extract", "set", "date-sum", "date", "order", "update"]
NEEDS_DATE = ("date-sum", "date")  # 정답에 날짜가 필요한 종류. 패킷 항목에는 날짜가 없다.
LOG_LINE = "worker step="


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def wilson(k: int, n: int) -> list:
    if n == 0:
        return [None, None]
    p = k / n
    centre = (p + Z95**2 / (2 * n)) / (1 + Z95**2 / n)
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n * n)) / (1 + Z95**2 / n)
    return [max(0.0, centre - half), min(1.0, centre + half)]


def rate(k: int, n: int) -> dict:
    return {"k": k, "n": n, "rate": k / n if n else None, "wilson95": wilson(k, n)}


def dist(values: np.ndarray) -> dict:
    p5, p25, p50, p75, p95 = np.percentile(values, [5, 25, 50, 75, 95])
    return {"n": int(values.size), "mean": float(values.mean()), "sd": float(values.std(ddof=1)),
            "median": float(p50), "p5": float(p5), "p25": float(p25), "p75": float(p75), "p95": float(p95),
            "min": float(values.min()), "max": float(values.max())}


def auc(positive: np.ndarray, negative: np.ndarray) -> float:
    """양성이 음성보다 높을 확률(같으면 0.5). 순위 합으로 계산한다."""
    values = np.concatenate([positive, negative])
    order = values.argsort(kind="mergesort")
    ranks = np.empty(values.size)
    sorted_values = values[order]
    start = 0
    for end in range(1, values.size + 1):
        if end == values.size or sorted_values[end] != sorted_values[start]:
            ranks[order[start:end]] = (start + end + 1) / 2
            start = end
    return float((ranks[:positive.size].sum() - positive.size * (positive.size + 1) / 2)
                 / (positive.size * negative.size))


def bootstrap(per_scenario_k: np.ndarray, per_scenario_n: np.ndarray, rng: np.random.Generator) -> list:
    """시나리오 단위 군집 부트스트랩 95% 백분위 구간."""
    size = per_scenario_k.size
    picks = rng.integers(0, size, size=(BOOTSTRAP_REPS, size))
    rates = per_scenario_k[picks].sum(axis=1) / per_scenario_n[picks].sum(axis=1)
    low, high = np.percentile(rates, [2.5, 97.5])
    return [float(low), float(high)]


# ---- 패킷 채우기 규칙(packet.rs, examples/packet/assemble.rs) ----

def tool_text(tool: dict) -> str:
    args = json.dumps(tool["args"], ensure_ascii=False, separators=(",", ":"))
    return f"{tool['tool']} {args}\n{tool['result']}"


def tool_path(tool: dict) -> str | None:
    for key in ("path", "file_path"):
        if isinstance(tool["args"].get(key), str):
            return tool["args"][key]
    return None


def tool_memo(tool: dict) -> str:
    path = tool_path(tool)
    if tool["tool"].lower() == "read" and path is not None:
        return f"{path} · all lines"
    return ""


def digest(tool: dict) -> str:
    head = tool_text(tool)[:DIGEST_HEAD_CHARS]
    memo = tool_memo(tool).split("\n")[0]
    return f"{memo}\n{head}" if memo else head


def fill(tools: dict, order: list, budget_chars: int) -> dict:
    """고른 순서대로 원문, 축약본, 경로 중 처음 들어가는 형태를 넣는다. {seq: (형태, 글)}"""
    item_cap = budget_chars * ITEM_SHARE_PERCENT // 100
    remaining = budget_chars
    chosen = {}
    for seq in order:
        tool = tools[seq]
        path = tool_path(tool)
        if path is not None and path.rsplit("/", 1)[-1] in PROVIDER_DOCS:
            continue
        text = tool_text(tool)
        forms = [("raw", text if len(text) <= item_cap else None), ("digest", digest(tool)), ("path", path)]
        for kind, form in forms:
            if form is not None and len(form) + len(ITEM_SEPARATOR) <= remaining:
                remaining -= len(form) + len(ITEM_SEPARATOR)
                chosen[seq] = (kind, form)
                break
    return chosen


def order_after_judge(rrf_order: list, probabilities: dict, threshold: float) -> list:
    """judge가 threshold 이상으로 남긴 항목을 확률 높은 순(같으면 RRF 순)으로, 답이 없는 항목을 RRF 순으로 뒤에 둔다."""
    kept = [(position, probabilities[seq], seq) for position, seq in enumerate(rrf_order)
            if seq in probabilities and probabilities[seq] >= threshold]
    kept.sort(key=lambda item: (-item[1], item[0]))
    unanswered = [seq for seq in rrf_order if seq not in probabilities]
    return [seq for _, _, seq in kept] + unanswered


def competing_budget(packet_text: str) -> int:
    """고정 구역 글자 수를 뺀 경쟁 구역 예산. 두 조건의 고정 구역은 같다."""
    marker = packet_text.find(f"## {COMPETING_TITLE}")
    fixed = packet_text if marker < 0 else packet_text[:marker]
    header = len(f"## {COMPETING_TITLE}{ITEM_SEPARATOR}")
    return max(BUDGET_TOKENS * CHARS_PER_TOKEN - len(fixed) - header, 0)


def evidence_line(tool: dict) -> str:
    return "\n".join(line for line in tool["result"].split("\n") if LOG_LINE not in line)


def question_kind(question: dict) -> str:
    """질문이 요구하는 정보의 종류."""
    text = question["text"]
    if question["qtype"] == "multi-session":
        return "date-sum" if re.search(r"\d{4}-\d{2}-\d{2}", text) else "set"
    if question["qtype"] == "temporal":
        return "date" if text.endswith("날짜는") else "order"
    return question["qtype"]


def write_csv(name: str, rows: list) -> None:
    path = RESULTS / "tables" / f"diagnosis-{name}.csv"
    with path.open("w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def main() -> None:
    scenarios = {}
    for path in sorted(RAW.glob("scenarios-*.jsonl")):
        for s in read_jsonl(path):
            scenarios[s["scenario_id"]] = s
    packets = {}
    for path in sorted(RAW.glob("packets-*.jsonl")):
        for p in read_jsonl(path):
            packets[p["scenario_id"]] = p
    with (PROCESSED / "trials.csv").open(encoding="utf-8", newline="") as f:
        trials = list(csv.DictReader(f))
    correct = {(t["condition"], t["scenario_id"], t["provider"], t["qid"]): t["correct"] == "1"
               for t in trials if t["correct"] != ""}
    ids = sorted(scenarios, key=lambda s: int(s[1:]))

    RESULTS.mkdir(exist_ok=True)
    (RESULTS / "tables").mkdir(exist_ok=True)

    tools_of, probs_of, budget_of = {}, {}, {}
    for sid in ids:
        record = scenarios[sid]["record"]
        tools_of[sid] = {r["seq"]: r for r in record if r["kind"] == "tool"}
        probs_of[sid] = {j["seq"]: j["probability"] for j in packets[sid]["judgments"]}
        budget_of[sid] = competing_budget(packets[sid]["packets"]["rrf-fallback"]["packet"])

    # 한 항목 상한(경쟁 구역 예산의 30%)을 넘는 후보 수
    item_caps = {sid: budget_of[sid] * ITEM_SHARE_PERCENT // 100 for sid in ids}
    item_limit = {"max_candidate_chars": max(len(tool_text(t)) for sid in ids for t in tools_of[sid].values()),
                  "min_item_cap_chars": min(item_caps.values()),
                  "candidates_over_cap": sum(len(tool_text(t)) > item_caps[sid]
                                             for sid in ids for t in tools_of[sid].values()),
                  "candidates": sum(len(tools_of[sid]) for sid in ids)}

    # 0. 채우기 규칙 재현 확인
    check = {"rrf-fallback": [0, 0], "judge-all": [0, 0]}
    for sid in ids:
        rrf = packets[sid]["packets"]["rrf-fallback"]["rrf_order"]
        for condition, order in (("rrf-fallback", rrf),
                                 ("judge-all", order_after_judge(rrf, probs_of[sid], ACTUAL_THRESHOLD))):
            chosen = fill(tools_of[sid], order, budget_of[sid])
            packet = packets[sid]["packets"][condition]
            check[condition][0] += int(sorted(chosen) == sorted(packet["included"]))
            marker = packet["packet"].find(f"## {COMPETING_TITLE}")
            body = packet["packet"][marker:] if marker >= 0 else ""
            rendered = (f"## {COMPETING_TITLE}{ITEM_SEPARATOR}"
                        + "".join(chosen[seq][1] + ITEM_SEPARATOR for seq in sorted(chosen))) if chosen else ""
            check[condition][1] += int(body == rendered)
    reproduction = {condition: {"included_matched": a, "text_matched": b, "scenarios": len(ids)}
                    for condition, (a, b) in check.items()}

    # 근거 항목 표(시나리오, 질문, 근거 항목)
    items = []
    for sid in ids:
        for q in scenarios[sid]["questions"]:
            for seq, relation in zip(q["evidence_seq"], q["evidence_relation"]):
                tool = tools_of[sid].get(seq)
                items.append({"sid": sid, "qid": q["qid"], "qtype": q["qtype"], "kind": question_kind(q),
                              "seq": seq, "relation": relation, "is_candidate": tool is not None,
                              "probability": probs_of[sid].get(seq)})
    evidence_seqs = {sid: {i["seq"] for i in items if i["sid"] == sid} for sid in ids}

    # 1~2. 후보와 Jev 확률 분포
    candidates = sum(1 for i in items if i["is_candidate"])
    judged = sum(1 for i in items if i["probability"] is not None)
    ev_probs = np.array([i["probability"] for i in items if i["probability"] is not None])
    non_probs = np.array([p for sid in ids for seq, p in probs_of[sid].items() if seq not in evidence_seqs[sid]])
    probability = {
        "evidence": dist(ev_probs), "non_evidence": dist(non_probs),
        "auc": auc(ev_probs, non_probs),
        "at_least": {f"{t:.2f}": {"evidence": rate(int((ev_probs >= t).sum()), ev_probs.size),
                                  "non_evidence": rate(int((non_probs >= t).sum()), non_probs.size)}
                     for t in THRESHOLDS},
        "by_relation": {rel: dist(np.array([i["probability"] for i in items if i["relation"] == rel]))
                        for rel in ("same", "translation", "synonym")},
        "by_kind": {kind: dist(np.array([i["probability"] for i in items if i["kind"] == kind]))
                    for kind in ("extract", "set", "date-sum", "date", "order", "update")},
    }
    per_scenario_rank = []
    for sid in ids:
        ranked = sorted(probs_of[sid], key=lambda seq: -probs_of[sid][seq])
        for seq in evidence_seqs[sid]:
            per_scenario_rank.append(ranked.index(seq) + 1)
    probability["evidence_rank_by_probability"] = {
        "median": float(np.median(per_scenario_rank)), "p25": float(np.percentile(per_scenario_rank, 25)),
        "p75": float(np.percentile(per_scenario_rank, 75)), "p95": float(np.percentile(per_scenario_rank, 95)),
        "within_top_24": rate(sum(r <= 24 for r in per_scenario_rank), len(per_scenario_rank))}

    # 3~4. 실제 judge-all 패킷의 단계별 손실(항목 단위)
    actual = {}
    for sid in ids:
        actual[sid] = {c: (set(packets[sid]["packets"][c]["included"]), packets[sid]["packets"][c]["packet"])
                       for c in ("judge-all", "rrf-fallback")}
    stage_items = defaultdict(int)
    for i in items:
        included, text = actual[i["sid"]]["judge-all"]
        i["in_packet"] = i["seq"] in included
        i["visible"] = i["in_packet"] and evidence_line(tools_of[i["sid"]][i["seq"]]) in text
        rrf_included, rrf_text = actual[i["sid"]]["rrf-fallback"]
        i["rrf_visible"] = i["seq"] in rrf_included and evidence_line(tools_of[i["sid"]][i["seq"]]) in rrf_text
        if not i["is_candidate"] or i["probability"] is None:
            stage = "1 후보에 없음"
        elif i["probability"] < ACTUAL_THRESHOLD:
            stage = "2 기준값 0.5 미달"
        elif not i["in_packet"]:
            stage = "3 기준 통과, 예산이나 한 항목 상한에서 빠짐"
        elif not i["visible"]:
            stage = "4 패킷에 들었지만 근거 줄이 잘림"
        else:
            stage = "5 근거 줄이 패킷에 있음"
        i["stage"] = stage
        stage_items[stage] += 1
    item_total = len(items)
    stage_item_rows = [{"stage": s, "items": stage_items[s], "rate": stage_items[s] / item_total}
                       for s in sorted(stage_items)]
    stage_item_rows.insert(0, {"stage": "전체 근거 항목", "items": item_total, "rate": 1.0})
    write_csv("stages-items", stage_item_rows)

    # 질문 단위: 질문에 필요한 근거 항목이 모두 보여야 답할 수 있다. update는 새 값 줄만 필요하다.
    questions = []
    for sid in ids:
        for q in scenarios[sid]["questions"]:
            if q["qtype"] == "abstain":
                continue
            mine = [i for i in items if i["sid"] == sid and i["qid"] == q["qid"]]
            if q["qtype"] == "update":
                mine = mine[1:]  # 답은 새 값이라 옛 값 줄 없이도 풀린다. 근거는 [옛 값, 새 값] 순이다.
            stages = sorted({i["stage"] for i in mine})
            questions.append({"sid": sid, "qid": q["qid"], "qtype": q["qtype"], "kind": question_kind(q),
                              "items": mine, "first_stage": stages[0],
                              "all_visible": all(i["visible"] for i in mine),
                              "rrf_all_visible": all(i["rrf_visible"] for i in mine)})

    # 틀린 질문마다 첫 손실 단계(provider 단위)
    wrong_stage = defaultdict(lambda: defaultdict(int))
    right_stage = defaultdict(lambda: defaultdict(int))
    for q in questions:
        for provider in ("claude", "codex"):
            ok = correct.get(("judge-all", q["sid"], provider, q["qid"]))
            if ok is None:
                continue
            if ok:
                right_stage[provider][q["first_stage"]] += 1
                continue
            stage = q["first_stage"]
            if stage.startswith("5") and q["kind"] in ("date", "date-sum"):
                stage = "5a 근거 줄은 있으나 시각이나 session이 패킷에 없어 풀 수 없음"
            elif stage.startswith("5"):
                stage = "5b 근거 줄이 있는데 받는 쪽이 못 맞힘"
            wrong_stage[provider][stage] += 1
    wrong_rows = []
    stage_names = sorted({s for p in wrong_stage.values() for s in p})
    for stage in stage_names:
        row = {"stage": stage}
        for provider in ("claude", "codex"):
            row[provider] = wrong_stage[provider][stage]
        row["total"] = row["claude"] + row["codex"]
        wrong_rows.append(row)
    wrong_rows.append({"stage": "틀린 질문 합계(abstain 제외)", "claude": sum(wrong_stage["claude"].values()),
                       "codex": sum(wrong_stage["codex"].values()),
                       "total": sum(sum(p.values()) for p in wrong_stage.values())})
    write_csv("stages-wrong-questions", wrong_rows)
    right_by_stage = {p: dict(v) for p, v in right_stage.items()}

    # 5. 근거가 모두 보일 때의 정답률(조건, provider, 질문 종류)
    visible_rows = []
    visible_summary = {}
    for condition, flag in (("judge-all", "all_visible"), ("rrf-fallback", "rrf_all_visible")):
        for provider in ("claude", "codex", "both"):
            providers = ("claude", "codex") if provider == "both" else (provider,)
            for kind in ("extract", "set", "date-sum", "date", "order", "update", "all"):
                for shown in (True, False):
                    k = n = 0
                    for q in questions:
                        if kind != "all" and q["kind"] != kind:
                            continue
                        if q[flag] != shown:
                            continue
                        for p in providers:
                            ok = correct.get((condition, q["sid"], p, q["qid"]))
                            if ok is not None:
                                n += 1
                                k += int(ok)
                    if n:
                        visible_rows.append({"condition": condition, "provider": provider, "kind": kind,
                                             "evidence_visible": int(shown), "correct": k, "n": n,
                                             "rate": k / n})
                        visible_summary[f"{condition}/{provider}/{kind}/{int(shown)}"] = rate(k, n)
    write_csv("recipient", visible_rows)

    # 6. 질문 종류별 필요한 정보와 패킷 형식
    date_strings = sum(len(re.findall(r"\d{4}-\d{2}-\d{2}", a[c][1])) for a in actual.values() for c in a)
    session_labels = sum(len(re.findall(r"(?i)\bsession\b|\bseq\b|ledger", a[c][1])) for a in actual.values() for c in a)
    format_rows = []
    kind_info = {
        "extract": "근거 줄 하나", "set": "근거 줄 둘(여러 session에 흩어짐)",
        "date-sum": "근거 줄 둘과 각 줄이 일어난 날짜", "date": "근거 줄 하나와 그 줄이 일어난 날짜",
        "order": "근거 줄 둘과 앞뒤 순서", "update": "같은 값의 옛 줄과 새 줄, 앞뒤 순서"}
    for kind, need in kind_info.items():
        mine = [q for q in questions if q["kind"] == kind]
        row = {"kind": kind, "needs": need, "questions": len(mine)}
        for condition, flag in (("judge-all", "all_visible"), ("rrf-fallback", "rrf_all_visible")):
            row[f"{condition}_evidence_visible"] = sum(q[flag] for q in mine)
            k = n = 0
            for q in mine:
                for p in ("claude", "codex"):
                    ok = correct.get((condition, q["sid"], p, q["qid"]))
                    if ok is not None:
                        n += 1
                        k += int(ok)
            row[f"{condition}_correct"] = k
            row[f"{condition}_n"] = n
        format_rows.append(row)
    write_csv("format", format_rows)
    format_summary = {"date_strings_in_96_packets": date_strings, "session_or_seq_labels_in_96_packets": session_labels,
                      "kinds": format_rows}

    # 반사실: 규칙마다 근거 포함률
    rng = np.random.default_rng(SEED)
    rule_rows, rule_summary = [], {}
    rules = [("threshold", t) for t in THRESHOLDS] + [("rrf", None)]
    for name, threshold in rules:
        label = "rrf-only" if name == "rrf" else f"threshold-{threshold:.2f}"
        item_in = item_visible = in_total = 0
        per_item_k = np.zeros(len(ids))
        per_item_n = np.zeros(len(ids))
        per_q_k = np.zeros(len(ids))
        per_q_n = np.zeros(len(ids))
        by_qtype = defaultdict(lambda: [0, 0])
        by_kind = defaultdict(lambda: [0, 0])
        answerable = 0
        size_items, size_tokens = [], []
        unmet_prob = 0
        for index, sid in enumerate(ids):
            rrf = packets[sid]["packets"]["rrf-fallback"]["rrf_order"]
            order = rrf if name == "rrf" else order_after_judge(rrf, probs_of[sid], threshold)
            chosen = fill(tools_of[sid], order, budget_of[sid])
            size_items.append(len(chosen))
            fixed = BUDGET_TOKENS * CHARS_PER_TOKEN - budget_of[sid] - len(f"## {COMPETING_TITLE}{ITEM_SEPARATOR}")
            size_tokens.append((fixed + len(f"## {COMPETING_TITLE}{ITEM_SEPARATOR}")
                                + sum(len(f) + len(ITEM_SEPARATOR) for _, f in chosen.values())) / CHARS_PER_TOKEN)
            visible = {seq for seq, (_, form) in chosen.items()
                       if evidence_line(tools_of[sid][seq]) in form}
            for i in (i for i in items if i["sid"] == sid):
                per_item_n[index] += 1
                item_in += int(i["seq"] in chosen)
                per_item_k[index] += int(i["seq"] in visible)
                item_visible += int(i["seq"] in visible)
                if name != "rrf" and i["probability"] >= threshold and i["seq"] not in chosen:
                    unmet_prob += 1
            for q in (q for q in questions if q["sid"] == sid):
                ok = all(i["seq"] in visible for i in q["items"])
                per_q_n[index] += 1
                per_q_k[index] += int(ok)
                by_qtype[q["qtype"]][0] += int(ok)
                by_qtype[q["qtype"]][1] += 1
                by_kind[q["kind"]][0] += int(ok)
                by_kind[q["kind"]][1] += 1
                answerable += int(ok and q["kind"] not in NEEDS_DATE)
        row = {"rule": label, "included_items_mean": float(np.mean(size_items)),
               "packet_tokens_mean": float(np.mean(size_tokens)),
               "evidence_items_in_packet": item_in, "evidence_lines_visible": item_visible,
               "evidence_items_total": item_total,
               "item_visible_rate": item_visible / item_total,
               "item_visible_ci95": bootstrap(per_item_k, per_item_n, rng),
               "question_all_visible": int(per_q_k.sum()), "questions": int(per_q_n.sum()),
               "question_all_visible_rate": per_q_k.sum() / per_q_n.sum(),
               "question_all_visible_ci95": bootstrap(per_q_k, per_q_n, rng),
               "passed_threshold_but_cut_by_budget": unmet_prob}
        row["question_answerable_in_current_format"] = answerable
        for qtype in QTYPES:
            row[f"question_{qtype}"] = f"{by_qtype[qtype][0]}/{by_qtype[qtype][1]}"
        for kind in KINDS:
            row[f"kind_{kind}"] = f"{by_kind[kind][0]}/{by_kind[kind][1]}"
        rule_summary[label] = {**row, "by_qtype": {q: rate(*by_qtype[q]) for q in QTYPES},
                               "by_kind": {k: rate(*by_kind[k]) for k in KINDS},
                               "answerable_rate": rate(answerable, int(per_q_n.sum()))}
        rule_rows.append({k: (round(v, 4) if isinstance(v, float) else (
            [round(x, 4) for x in v] if isinstance(v, list) else v)) for k, v in row.items()})
    write_csv("counterfactual", rule_rows)

    # 근거를 모두 담은 질문 중 같은 패킷에 비근거 항목이 얼마나 함께 들었나는 포함 항목 수로 갈음한다.
    summary = {
        "seed": SEED, "bootstrap_reps": BOOTSTRAP_REPS, "scenarios": len(ids),
        "reproduction": reproduction, "item_limit": item_limit,
        "evidence_items": {"total": item_total, "candidates": candidates, "judged": judged},
        "probability": probability,
        "stages_items": {r["stage"]: {"items": r["items"], "rate": r["rate"]} for r in stage_item_rows[1:]},
        "stages_wrong_questions": {r["stage"]: {"claude": r["claude"], "codex": r["codex"], "total": r["total"],
                                                "rate": r["total"] / wrong_rows[-1]["total"]} for r in wrong_rows},
        "correct_questions_by_first_stage": right_by_stage,
        "recipient": visible_summary,
        "format": format_summary,
        "counterfactual": rule_summary,
    }
    with (RESULTS / "diagnosis-summary.json").open("w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
