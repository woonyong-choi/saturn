"""raw JSON Lines를 분석 가능한 CSV로 펼친다."""
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]
MAIN_REPO = Path.home() / "workspace" / "oss" / "saturn"
RAW = ROOT / "data" / "raw"
PROCESSED = ROOT / "data" / "processed"


def current_run_id() -> str | None:
    env = ROOT / "env.json"
    if not env.exists():
        return None
    return json.loads(env.read_text(encoding="utf-8")).get("run_id")


def rows(prefix: str) -> list[dict]:
    result = []
    paths = sorted(RAW.glob(f"{prefix}-*.jsonl"))
    if paths:
        with paths[-1].open(encoding="utf-8") as source:
            result.extend(json.loads(line) for line in source if line.strip())
    return result


def write(name: str, values: list[dict], fields: list[str]):
    PROCESSED.mkdir(parents=True, exist_ok=True)
    with (PROCESSED / f"{name}.csv").open("w", encoding="utf-8", newline="") as target:
        writer = csv.DictWriter(target, fieldnames=fields)
        writer.writeheader()
        writer.writerows({field: value for field, value in row.items() if field in fields} for row in values)


def private_messages(row: dict) -> list[dict]:
    path = Path(row.get("private_log", "")).expanduser()
    if not path.is_absolute():
        path = MAIN_REPO / path
    if not path.exists():
        return []
    messages = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
            if event.get("direction") == "in":
                messages.append(json.loads(event["line"]))
        except (json.JSONDecodeError, KeyError, TypeError):
            continue
    return messages


def command_values(messages: list[dict]) -> list[str]:
    values = []

    def visit(value):
        if isinstance(value, dict):
            if value.get("name") == "Bash":
                command = (value.get("input") or {}).get("command")
                if isinstance(command, str):
                    values.append(command)
            if value.get("type") == "commandExecution" and isinstance(value.get("command"), str):
                values.append(value["command"])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit(messages)
    return list(dict.fromkeys(values))


def state_check(command: str) -> bool:
    return command.strip().startswith(("test", "ls", "stat", "find", "pwd", "git status", "git diff"))


def text_values(value) -> list[str]:
    values = []
    if isinstance(value, dict):
        for key in ("text", "message", "error", "status"):
            if isinstance(value.get(key), str):
                values.append(value[key])
        for child in value.values():
            values.extend(text_values(child))
    elif isinstance(value, list):
        for child in value:
            values.extend(text_values(child))
    return values


def main():
    exp1 = rows("exp1")
    exp2 = rows("exp2")
    exp3 = rows("exp3")
    exp4 = rows("exp4")
    for row in exp2:
        commands = command_values(private_messages(row))
        row["commands"] = commands
        row["state_checked_first"] = int(bool(commands and state_check(commands[0])))
        row["duplicate_touch"] = int(any("touch worktree-touch.txt" in command for command in commands))
    question_rows = []
    for row in exp1:
        row["included_json"] = json.dumps(row.get("included", []), ensure_ascii=False)
        row["manipulation_check_json"] = json.dumps(row.get("manipulation_check", {}), ensure_ascii=False)
        for question in row.get("question_results", []):
            question_rows.append({"run_id": row["run_id"], "trial_id": row["trial_id"], "scenario_id": row["scenario_id"],
                                  "provider": row["provider"], "condition": row["condition"], "qid": question["qid"],
                                  "qtype": question["qtype"], "correct": question["correct"]})
    write("exp1_questions", question_rows, ["run_id", "trial_id", "scenario_id", "provider", "condition", "qid", "qtype", "correct"])
    write("exp1_trials", exp1, ["run_id", "trial_id", "condition", "ts_utc", "scenario_id", "provider", "strategy", "packet_tokens", "tokenizer", "budget_tokens", "included_json", "manipulation_check_json", "status", "exit_code", "elapsed_s"])
    write("exp2_trials", exp2, ["run_id", "trial_id", "condition", "ts_utc", "provider", "state_checked_first", "duplicate_touch", "status"])
    request_rows = []
    for row in exp3:
        if "item/tool/requestUserInput" in row.get("request_method", []):
            # 수집 당시 지표는 요청 도착만 봤다. 모델 후속 출력에 드라이버의 답이 반영됐는지로 다시 센다.
            row["round_trip"] = int("experiment-answer" in json.dumps(row.get("server_observation"), ensure_ascii=False))
        methods = row.get("request_method", [])
        for index, method in enumerate(methods):
            request_rows.append({"run_id": row["run_id"], "trial_id": row["trial_id"], "condition": row["condition"],
                                 "provider": row["provider"], "request_method": method,
                                 "request": json.dumps((row.get("request_params") or [None])[index], ensure_ascii=False),
                                 "round_trip": row.get("round_trip", 0)})
    write("exp3_requests", request_rows, ["run_id", "trial_id", "condition", "provider", "request_method", "request", "round_trip"])
    stop_rows = []
    for row in exp4:
        display_text = row.get("display_text")
        if isinstance(display_text, list):
            display_text = " | ".join(display_text)
        elif not isinstance(display_text, str):
            display_text = " | ".join(text_values(row.get("events", [])))
        stop_rows.append({"run_id": row["run_id"], "trial_id": row["trial_id"], "condition": row["condition"],
                          "provider": row["provider"], "stop_kind": row["stop_kind"],
                          "display_text": display_text,
                          "screen_path": row.get("screen_path"), "raw_screen_path": row.get("raw_screen_path"),
                          "status": row.get("status")})
    write("exp4_stops", stop_rows, ["run_id", "trial_id", "condition", "provider", "stop_kind", "display_text", "screen_path", "raw_screen_path", "status"])


if __name__ == "__main__":
    main()
