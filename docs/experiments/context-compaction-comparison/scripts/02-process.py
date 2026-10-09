"""로컬 원응답을 대응 질문 단위로 채점하고 공개 집계만 만든다."""

from __future__ import annotations

import json
import random
import re
from statistics import median
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
EXP = Path(__file__).resolve().parent.parent
LOCAL = ROOT / ".local/experiments/context-compaction-comparison/sessions-20261005T115947Z-145293d.jsonl"
SCENARIOS = ROOT / "docs/experiments/handoff-packet-quality-v2/data/raw/scenarios-20261001T185207Z-aa1e13a.jsonl"


def normalize(value: object) -> str:
    return re.sub(r"[\s,]", "", str(value).lower())


def grade(question: dict, answer: dict | None) -> int:
    if answer is None:
        return 0
    if question["qtype"] == "abstain":
        return int(bool(answer.get("unknown")))
    if answer.get("unknown"):
        return 0
    said = normalize(answer.get("answer", ""))
    return int(all(normalize(gold) in said for gold in question["gold"])
               and not any(normalize(stale) in said for stale in question["stale"]))


def answers_of(reply: dict) -> dict:
    if reply.get("code") != 0:
        return {}
    try:
        data = json.loads(reply["stdout"])
        if data.get("num_turns", 1) != 1:
            return {}
        text = data["result"]
        parsed = json.loads(text[text.index("["):text.rindex("]") + 1])
        return {str(item.get("id")): item for item in parsed}
    except (KeyError, ValueError, TypeError, json.JSONDecodeError):
        return {}


def main() -> None:
    source = {row["scenario_id"]: row for row in map(json.loads, SCENARIOS.read_text().splitlines())}
    rows = list(map(json.loads, LOCAL.read_text().splitlines()))
    if len({row["scenario_id"] for row in rows}) != len(rows):
        raise ValueError("중복 시나리오")
    conditions = ["saturn-packet", "fast-jev"]
    pairs = []
    sessions = []
    for row in rows:
        scenario = source[row["scenario_id"]]
        answers = {condition: answers_of(row.get("conditions", {}).get(condition, {})) for condition in conditions}
        session = {"scenario_id": row["scenario_id"], "compaction": row.get("compaction", {}).get("stats"),
                   "jev_usage": row.get("compaction", {}).get("jev_usage"), "error": row.get("error")}
        for condition in conditions:
            reply = row.get("conditions", {}).get(condition, {})
            session[condition] = {"context_chars": reply.get("context_chars"), "elapsed_s": reply.get("elapsed_s"), "answered": len(answers[condition])}
        sessions.append(session)
        for question in scenario["questions"]:
            scores = {condition: grade(question, answers[condition].get(question["qid"])) for condition in conditions}
            pairs.append({"scenario_id": row["scenario_id"], "qid": question["qid"], "qtype": question["qtype"], **scores})
    by_scenario = {sid: [p for p in pairs if p["scenario_id"] == sid] for sid in source if any(p["scenario_id"] == sid for p in pairs)}
    rng = random.Random(409)
    ids = list(by_scenario)
    differences = []
    for _ in range(10_000):
        sampled = [pair for sid in rng.choices(ids, k=len(ids)) for pair in by_scenario[sid]]
        differences.append(sum(p["saturn-packet"] - p["fast-jev"] for p in sampled) / len(sampled))
    differences.sort()
    n = len(pairs)
    summary = {
        "scenarios": len(rows), "questions_per_condition": n,
        "correct": {condition: sum(p[condition] for p in pairs) for condition in conditions},
        "accuracy": {condition: sum(p[condition] for p in pairs) / n for condition in conditions},
        "difference": sum(p["saturn-packet"] - p["fast-jev"] for p in pairs) / n,
        "difference_ci95": [differences[250], differences[9749]],
        "discordant": {"packet_only": sum(p["saturn-packet"] == 1 and p["fast-jev"] == 0 for p in pairs),
                       "fast_only": sum(p["saturn-packet"] == 0 and p["fast-jev"] == 1 for p in pairs)},
        "by_type": {kind: {condition: {"correct": sum(p[condition] for p in pairs if p["qtype"] == kind),
                                       "total": sum(p["qtype"] == kind for p in pairs)} for condition in conditions}
                    for kind in sorted({p["qtype"] for p in pairs})},
        "compaction": {"calls_dropped": sum((s["compaction"] or {}).get("callsDropped", 0) for s in sessions),
                       "results_dropped": sum((s["compaction"] or {}).get("resultsDropped", 0) for s in sessions),
                       "calls_kept": sum((s["compaction"] or {}).get("kept", 0) for s in sessions),
                       "pinned": sum((s["compaction"] or {}).get("pinned", 0) for s in sessions)},
        "context_chars_median": {condition: median(s[condition]["context_chars"] for s in sessions if s[condition]["context_chars"] is not None) for condition in conditions},
        "claude_elapsed_s_total": {condition: round(sum(s[condition]["elapsed_s"] or 0 for s in sessions), 3) for condition in conditions},
        "jev_usage_total": {key: sum((s["jev_usage"] or {}).get(key, 0) for s in sessions) for key in ("requests", "input_tokens", "output_tokens")},
        "errors": [s["scenario_id"] for s in sessions if s["error"] or any(s[c]["answered"] != 10 for c in conditions)],
        "sessions": sessions,
    }
    results = EXP / "results"
    results.mkdir(exist_ok=True)
    (results / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    print(f"{len(rows)} scenarios, {n} paired questions, difference {summary['difference']:.3f}")


if __name__ == "__main__":
    main()
