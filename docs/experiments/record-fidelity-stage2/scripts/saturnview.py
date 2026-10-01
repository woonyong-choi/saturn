"""Saturn 이벤트에서 기록을 만들고, 원시 줄에서 실제로 거친 근거를 찾는다.

이벤트는 수정된 변환(#210, #214, #215)이 낸 값만 쓴다. 도구 종류, 경로, 줄 수, 종료 코드는 이벤트의 필드에서 읽는다.
"""
import json
import re

TOOL_NAME = {"FileRead": "read", "FileEdit": "edit", "Shell": "shell", "TestRun": "test", "Other": "other"}
EDIT_TOOLS = ("Edit", "MultiEdit", "Write")


def build_records(events):
    """ToolCall과 ToolResult를 call_id로 이어 도구 기록을, Text를 에이전트 기록으로 만든다. 추론(Reasoning) 항목은 기록에 넣지 않는다."""
    records = []
    by_call = {}
    for event in events:
        (name, body), = event.items()
        if name == "ToolCall":
            detail = body["detail"]
            if detail["category"] == "Reasoning":
                continue
            activity = body["activity"]
            record = {
                "type": "tool", "call_id": body["call_id"], "kind": detail["category"],
                "command": activity["RunningCommand"]["command"] if isinstance(activity, dict) else None,
                "paths": detail["paths"], "read_lines": detail["read_lines"], "changed": detail["changed"],
                "exit_code": None, "output": None,
            }
            by_call[body["call_id"]] = record
            records.append(record)
        elif name == "ToolResult" and body["call_id"] in by_call:
            by_call[body["call_id"]]["output"] = body["output"]
            by_call[body["call_id"]]["exit_code"] = body["exit_code"]
        elif name == "Text":
            if records and records[-1]["type"] == "agent":
                records[-1]["text"] += body["text"]
            else:
                records.append({"type": "agent", "text": body["text"]})
    return records


def tool_args(record):
    """패킷 예제에 넘기는 인자. 이벤트에 있는 값만 옮긴다."""
    args = {}
    if record["paths"]:
        args["path"] = record["paths"][0]
    if record["command"] is not None:
        args["command"] = record["command"]
    if record["exit_code"] is not None:
        args["exit_code"] = record["exit_code"]
    if record["changed"]:
        args["added"] = record["changed"]["added"]
        args["removed"] = record["changed"]["removed"]
    if record["read_lines"]:
        args["read_lines"] = record["read_lines"]
    return args


def test_counts_of(output):
    names = set(re.findall(r"^FAIL: (test_\w+)", output or "", re.M))
    ran = re.search(r"^Ran (\d+) tests?", output or "", re.M)
    failed = re.search(r"FAILED \((?:failures|errors)=(\d+)", output or "")
    if not ran or not failed:
        return None
    return {"failed": int(failed.group(1)), "passed": int(ran.group(1)) - int(failed.group(1)), "names": names}


def path_recovered(item, record):
    return any(path.endswith(item["path"]) for path in record["paths"])


def memo_fields_ok(item, record, task_item_facts):
    """메모 틀의 종류별 필드를 이벤트 필드에서 얻어 정답과 대조한다."""
    if item["kind"] in ("Shell", "TestRun"):
        command_ok = record["command"] is not None and item["command"] in record["command"]
        exit_ok = record["exit_code"] == item["exit_code"]
        if item["kind"] == "Shell":
            return command_ok and exit_ok
        counts = test_counts_of(record["output"])
        expected = {"failed": 2, "passed": 2, "names": {fact["marker"] for fact in task_item_facts}}
        return command_ok and exit_ok and counts == expected
    if item["kind"] == "FileRead":
        return path_recovered(item, record)
    changed = record["changed"] or {}
    return path_recovered(item, record) and changed.get("added") == item["added"] and changed.get("removed") == item["removed"]


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
