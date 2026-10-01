"""Saturn 이벤트에서 기록을 만들고, 원시 줄에서 실제로 거친 근거를 찾는다.

Saturn 이벤트는 engine의 변환 코드가 낸 값만 쓴다. 이벤트에 없는 값(경로, 종료 코드, 줄 수)은 아래 규칙으로 얻을 수 있는 만큼만 얻고, 못 얻으면 없음으로 둔다.
"""
import json
import re

KIND_OF_ACTIVITY = {"ReadingFile": "FileRead", "EditingFile": "FileEdit"}
TOOL_NAME = {"FileRead": "read", "FileEdit": "edit", "Shell": "shell", "Other": "other"}
EDIT_TOOLS = ("Edit", "MultiEdit", "Write")
EXIT_PREFIX = re.compile(r"\AExit code (\d+)")


def read_events(path):
    events = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                events.append(json.loads(line))
    return events


def build_records(events):
    """ToolCall과 ToolResult를 call_id로 이어 도구 기록을, Text를 에이전트 기록으로 만든다."""
    records = []
    by_call = {}
    for event in events:
        (name, body), = event.items() if len(event) == 1 else (("", {}),)
        if name == "ToolCall":
            activity = body["activity"]
            command = activity["RunningCommand"]["command"] if isinstance(activity, dict) else None
            kind = "Shell" if command is not None else KIND_OF_ACTIVITY.get(activity, "Other")
            record = {"type": "tool", "call_id": body["call_id"], "activity": activity, "kind": kind, "command": command, "output": None}
            by_call[body["call_id"]] = record
            records.append(record)
        elif name == "ToolResult" and body["call_id"] in by_call:
            by_call[body["call_id"]]["output"] = body["output"]
        elif name == "Text":
            if records and records[-1]["type"] == "agent":
                records[-1]["text"] += body["text"]
            else:
                records.append({"type": "agent", "text": body["text"]})
    return records


def exit_code_of(output):
    match = EXIT_PREFIX.match(output or "")
    return int(match.group(1)) if match else None


def test_counts_of(output):
    names = set(re.findall(r"^FAIL: (test_\w+)", output or "", re.M))
    ran = re.search(r"^Ran (\d+) tests?", output or "", re.M)
    failed = re.search(r"FAILED \((?:failures|errors)=(\d+)", output or "")
    if not ran or not failed:
        return None
    return {"failed": int(failed.group(1)), "passed": int(ran.group(1)) - int(failed.group(1)), "names": names}


def memo_fields_ok(item, record, task_item_facts):
    """설계 문서의 메모 틀에서 종류별 필드를 이벤트로 얻을 수 있는 만큼 얻어 정답과 대조한다."""
    output = record["output"] or ""
    if item["kind"] in ("Shell", "TestRun"):
        command_ok = record["command"] is not None and item["command"] in record["command"]
        exit_ok = exit_code_of(output) == item["exit_code"]
        if item["kind"] == "Shell":
            return command_ok and exit_ok
        counts = test_counts_of(output)
        expected = {"failed": 2, "passed": 2, "names": {fact["marker"] for fact in task_item_facts}}
        return command_ok and exit_ok and counts == expected
    if item["kind"] == "FileRead":
        return path_recovered(item, record)
    # FileEdit: 더한 줄과 지운 줄은 이벤트에 없다
    return False


def path_recovered(item, record):
    text = (record["command"] or "") + "\n" + (record["output"] or "")
    return item["path"] in text


def raw_tool_results(provider, rows):
    texts = []
    for row in rows:
        if row["dir"] != "out":
            continue
        message = json.loads(row["line"])
        if provider == "claude" and message.get("type") == "user":
            for part in (message.get("message") or {}).get("content") or []:
                if isinstance(part, dict) and part.get("type") == "tool_result":
                    content = part.get("content")
                    texts.append(content if isinstance(content, str) else "".join(p.get("text", "") for p in content or []))
        elif provider == "codex" and message.get("method") == "item/completed":
            item = message["params"]["item"]
            if item.get("type") == "commandExecution":
                texts.append(item.get("aggregatedOutput") or "")
    return texts


def raw_edited_paths(provider, rows):
    paths = []
    for row in rows:
        if row["dir"] != "out":
            continue
        message = json.loads(row["line"])
        if provider == "claude" and message.get("type") == "assistant":
            for part in (message.get("message") or {}).get("content") or []:
                if isinstance(part, dict) and part.get("type") == "tool_use" and part.get("name") in EDIT_TOOLS:
                    paths.append(part["input"].get("file_path", ""))
        elif provider == "codex" and message.get("method") == "item/completed":
            item = message["params"]["item"]
            if item.get("type") == "fileChange":
                paths.extend(change.get("path", "") for change in item.get("changes", []))
    return paths
