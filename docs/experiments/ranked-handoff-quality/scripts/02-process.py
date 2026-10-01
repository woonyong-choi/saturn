"""raw/의 세션 답을 질문 단위로 채점해 processed/에 쓴다.

사용: python3 scripts/02-process.py (실험 폴더에서 실행)
조건 이름은 채점에 쓰지 않고 출력 열로만 옮긴다.
"""
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

TOP_N = 10
EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"
PROCESSED = EXPERIMENT / "data" / "processed"
SEMANTIC = {"translation", "synonym"}


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def normalize(text: str) -> str:
    return re.sub(r"[\s,]", "", str(text).lower())


def reply_text(provider: str, stdout: str) -> tuple[str, int]:
    """provider 출력에서 마지막 답 글과 도구 호출 수를 꺼낸다."""
    if provider == "claude":
        data = json.loads(stdout)
        return data.get("result", ""), max(int(data.get("num_turns", 1)) - 1, 0)
    text, tools = "", 0
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") or {}
        kind = item.get("type", "")
        if event.get("type") == "item.completed" and kind == "agent_message":
            text = item.get("text", "")
        elif event.get("type") == "item.started" and kind != "agent_message" and kind != "reasoning":
            tools += 1
    return text, tools


def parse_answers(text: str) -> dict:
    start, end = text.find("["), text.rfind("]")
    if start < 0 or end < start:
        raise ValueError("답 JSON 배열 없음")
    return {str(a.get("id")): a for a in json.loads(text[start:end + 1])}


def grade(question: dict, answer: dict | None) -> int:
    if answer is None:
        return 0
    unknown = bool(answer.get("unknown"))
    if question["qtype"] == "abstain":
        return int(unknown)
    said = normalize(answer.get("answer", ""))
    if unknown or not all(normalize(g) in said for g in question["gold"]):
        return 0
    return int(not any(normalize(s) in said for s in question["stale"]))


def main() -> None:
    session_files = sorted(RAW.glob("sessions-*.jsonl"))
    if not session_files:
        print("02-process: data/raw에 sessions 파일이 없다", file=sys.stderr)
        sys.exit(2)
    scenarios = {}
    for path in sorted(RAW.glob("scenarios-*.jsonl")):
        for s in read_jsonl(path):
            scenarios[(s["run_id"], s["scenario_id"])] = s
    PROCESSED.mkdir(parents=True, exist_ok=True)
    trials, sessions, misses = [], [], []
    ranked = set()
    for path in session_files:
        for row in read_jsonl(path):
            scenario = scenarios[(row["run_id"], row["scenario_id"])]
            status, answers, tools = "ok", {}, 0
            if row["exit_code"] != 0:
                status = "failed"
            else:
                try:
                    text, tools = reply_text(row["provider"], row["stdout"])
                    answers = parse_answers(text)
                except (ValueError, json.JSONDecodeError):
                    status = "unparsed"
                if status == "ok" and tools > 0:
                    status = "tool_used"
            sessions.append({"run_id": row["run_id"], "trial_id": row["trial_id"],
                             "scenario_id": row["scenario_id"], "provider": row["provider"],
                             "condition": row["condition"], "status": status, "attempt": row["attempt"],
                             "packet_tokens": row["packet_tokens"], "judge_calls": row["judge_calls"],
                             "judge_failures": row["judge_failures"], "elapsed_s": row["elapsed_s"]})
            for q in scenario["questions"]:
                trials.append({"run_id": row["run_id"], "trial_id": f"{row['trial_id']}-{q['qid']}",
                               "unit": f"{row['scenario_id']}-{row['provider']}-{q['qid']}",
                               "scenario_id": row["scenario_id"], "provider": row["provider"],
                               "qid": q["qid"], "qtype": q["qtype"], "condition": row["condition"],
                               "correct": grade(q, answers.get(q["qid"])) if status == "ok" else ""})
            if row["condition"] == "rrf-only" and (row["run_id"], row["scenario_id"]) not in ranked:
                ranked.add((row["run_id"], row["scenario_id"]))
                top = set(row["rrf_order"][:TOP_N])
                for q in scenario["questions"]:
                    for seq, relation in zip(q["evidence_seq"], q["evidence_relation"]):
                        misses.append({"run_id": row["run_id"], "scenario_id": row["scenario_id"],
                                       "qid": q["qid"], "seq": seq, "relation": relation,
                                       "in_top_n": int(seq in top), "semantic": int(relation in SEMANTIC)})
    for name, rows in (("trials", trials), ("sessions", sessions), ("evidence", misses)):
        with (PROCESSED / f"{name}.csv").open("w", encoding="utf-8", newline="") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]) if rows else ["run_id"])
            writer.writeheader()
            writer.writerows(rows)


if __name__ == "__main__":
    main()
