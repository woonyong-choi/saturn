"""data/raw/의 JSONL을 읽어 data/processed/에 압축 사건 표와 질문 쌍 표를 만든다.

사용: python3 scripts/02-process.py (실험 폴더에서 실행)
"""
import csv
import json
import sys
from pathlib import Path

EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = EXPERIMENT_DIR / "data" / "raw"
PROCESSED_DIR = EXPERIMENT_DIR / "data" / "processed"
CONDITIONS = ("claude-summary", "saturn-packet")


def read_jsonl(prefix: str) -> list[dict]:
    rows = []
    for path in sorted(RAW_DIR.glob(f"{prefix}-*.jsonl")):
        rows += [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    return rows


def normalize(answer: object) -> str:
    text = "" if answer is None else str(answer)
    text = text.strip().strip("`").strip().rstrip(".").strip().lower()
    return text[2:] if text.startswith("./") else text


def flag(value: bool) -> str:
    return "true" if value else "false"


def write_csv(name: str, header: list[str], rows: list[list]) -> None:
    PROCESSED_DIR.mkdir(parents=True, exist_ok=True)
    with (PROCESSED_DIR / name).open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def process_claude(rows: list[dict]) -> set[str]:
    compactions = [r for r in rows if r["kind"] == "compaction"]
    scenarios = [r for r in rows if r["kind"] == "scenario"]
    compaction_rows = []
    for r in sorted(compactions, key=lambda r: (r["run_id"], r["trial_id"])):
        success = bool(r["local_boundary_found"] and r["summary_text"] and r["stream_boundary_record_no"] is not None)
        compaction_rows.append([
            r["run_id"], r["trial_id"], r["scenario_id"], r["trigger"] or "", flag(r["local_boundary_found"]),
            len(r["summary_text"]), r["stream_match_count"],
            "" if r["stream_boundary_record_no"] is None else r["stream_boundary_record_no"], flag(success),
        ])
    write_csv("compactions.csv", [
        "run_id", "trial_id", "scenario_id", "trigger", "local_boundary_found", "summary_chars",
        "stream_match_count", "stream_boundary_record_no", "read_success",
    ], compaction_rows)
    write_csv("scenarios.csv", ["run_id", "scenario_id", "turns", "compaction_count", "error"], [
        [r["run_id"], r["scenario_id"], r["turns"], r["compaction_count"], r["error"] or ""]
        for r in sorted(scenarios, key=lambda r: (r["run_id"], r["scenario_id"]))
    ])
    return {r["scenario_id"] for r in scenarios if r["error"]}


def process_codex(rows: list[dict], failed_scenarios: set[str]) -> None:
    by_pair: dict[tuple[str, str, str], dict[str, dict]] = {}
    for r in rows:
        by_pair.setdefault((r["run_id"], r["scenario_id"], r["question_id"]), {})[r["condition"]] = r
    pair_rows = []
    for key in sorted(by_pair):
        pair = by_pair[key]
        run_id, scenario_id, question_id = key
        excluded = ""
        if scenario_id in failed_scenarios:
            excluded = "claude_failed"
        elif any(c not in pair for c in CONDITIONS):
            excluded = "missing_condition"
        elif any(pair[c]["tool_used"] for c in CONDITIONS):
            excluded = "tool_used"
        elif any((pair[c]["error"] or "").startswith("codex 종료 코드") for c in CONDITIONS):
            excluded = "codex_failed"
        first = next(iter(pair.values()))
        correct = {c: flag(c in pair and normalize(pair[c]["answer"]) == normalize(pair[c]["expected"])) for c in CONDITIONS}
        fallback = flag(any(pair[c]["summary_fallback"] for c in CONDITIONS if c in pair))
        pair_rows.append([
            run_id, scenario_id, question_id, first["question_kind"], correct["claude-summary"], correct["saturn-packet"],
            fallback, pair.get("claude-summary", {}).get("packet_chars", ""), pair.get("saturn-packet", {}).get("packet_chars", ""),
            excluded,
        ])
    write_csv("answers.csv", [
        "run_id", "scenario_id", "question_id", "question_kind", "correct_claude_summary", "correct_saturn_packet",
        "summary_fallback", "packet_chars_claude_summary", "packet_chars_saturn_packet", "excluded",
    ], pair_rows)


def main() -> int:
    claude_rows = read_jsonl("claude")
    if not claude_rows:
        print("처리 불가: data/raw/claude-*.jsonl이 없다", file=sys.stderr)
        return 1
    failed = process_claude(claude_rows)
    process_codex(read_jsonl("codex"), failed)
    return 0


if __name__ == "__main__":
    sys.exit(main())
