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

EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"
PROCESSED = EXPERIMENT / "data" / "processed"
PACKET_CONDITIONS = ("last-input", "rule", "summary")


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
        elif event.get("type") == "item.started" and kind not in ("agent_message", "reasoning"):
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
    if question["qtype"] == "unknown-result":
        said = normalize(answer.get("answer", ""))
        return int("모른다" in said or "확인전" in said or "unknown" in said)
    said = normalize(answer.get("answer", ""))
    if unknown or not all(normalize(g) in said for g in question["gold"]):
        return 0
    return int(not any(normalize(s) in said for s in question["stale"]))


def subtype(question: dict) -> str:
    """multi-session과 temporal 질문 둘씩을 질문 문구로 가른다."""
    text = question["text"]
    if question["qtype"] == "multi-session":
        return "sum" if "합" in text else "codes"
    if question["qtype"] == "temporal":
        return "first" if "먼저" in text else "date"
    return ""


def write_csv(name: str, rows: list[dict]) -> None:
    with (PROCESSED / f"{name}.csv").open("w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]) if rows else ["run_id"])
        writer.writeheader()
        writer.writerows(rows)


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
    trials, sessions, evidence, packets = [], [], [], []
    for path in sorted(RAW.glob("packets-*.jsonl")):
        for row in read_jsonl(path):
            scenario = scenarios[(row["run_id"], row["scenario_id"])]
            record_text = {record["seq"]: record.get("text", "") for record in scenario["record"]}
            for condition in PACKET_CONDITIONS:
                packet = row["packets"][condition]
                included = set(packet.get("included", []))
                packets.append({"run_id": row["run_id"], "scenario_id": row["scenario_id"], "condition": condition,
                                "packet_tokens": int(packet["tokens"]), "included_items": len(included),
                                "summary_tokens": int(packet.get("summary_total_tokens", 0)),
                                "summary_calls": int(row["summary_calls"] if condition == "summary" else 0)})
                for q in scenario["questions"]:
                    for seq, relation in zip(q["evidence_seq"], q["evidence_relation"]):
                        evidence.append({"run_id": row["run_id"], "scenario_id": row["scenario_id"],
                                         "condition": condition, "qid": q["qid"], "qtype": q["qtype"],
                                         "seq": seq, "relation": relation,
                                         "in_packet": int(seq in included or (
                                             bool(record_text[seq]) and record_text[seq] in packet["packet"]
                                         ))})
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
                except (ValueError, json.JSONDecodeError, AttributeError):
                    status = "unparsed"
                if status == "ok" and tools > 0:
                    status = "tool_used"
            sessions.append({"run_id": row["run_id"], "trial_id": row["trial_id"],
                             "scenario_id": row["scenario_id"], "provider": row["provider"],
                             "condition": row["condition"], "status": status, "attempt": row["attempt"],
                             "packet_tokens": row["packet_tokens"], "prompt_tokens": row["prompt_tokens"],
                             "elapsed_s": row["elapsed_s"]})
            for q in scenario["questions"]:
                trials.append({"run_id": row["run_id"], "trial_id": f"{row['trial_id']}-{q['qid']}",
                               "unit": f"{row['scenario_id']}-{row['provider']}-{q['qid']}",
                               "scenario_id": row["scenario_id"], "provider": row["provider"],
                               "qid": q["qid"], "qtype": q["qtype"], "qsub": subtype(q), "condition": row["condition"],
                               "correct": grade(q, answers.get(q["qid"])) if status == "ok" else ""})
    for name, rows in (("trials", trials), ("sessions", sessions), ("evidence", evidence), ("packets", packets)):
        write_csv(name, rows)


if __name__ == "__main__":
    main()
