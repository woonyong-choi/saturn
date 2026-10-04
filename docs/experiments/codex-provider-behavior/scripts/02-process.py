#!/usr/bin/env python3
"""raw 시험 행에서 시험마다 판정용 관측값을 뽑아 processed/trials.csv와 processed/trials.jsonl을 만든다.

raw의 이벤트·승인 기록·파일 효과만 입력으로 쓴다. 가설별 판정은 03-analyze.py가 한다.
"""
from __future__ import annotations

import csv
import glob
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUT = ROOT / "data" / "processed"
NO_ACTIVE_PATTERNS = ("no active turn", "not active", "expected turn", "turn mismatch", "no turn")
META_NAME_KEYS = ("tool_name", "tool", "toolName", "name")
DENIAL = ("read-only file system", "operation not permitted", "permission denied")
TOOL_RE = re.compile(r'tool "([^"]+)"')


def load_rows() -> list[dict]:
    rows = []
    for path in sorted(glob.glob(str(RAW / "*.jsonl"))):
        if path.endswith("-calls.jsonl"):
            continue
        for line in Path(path).read_text(encoding="utf-8").splitlines():
            if line.strip():
                rows.append(json.loads(line))
    return rows


def inbound(row):
    return [(e["ms"], e["msg"]) for e in row.get("events", []) if e["dir"] == "in"]


def outbound(row):
    return [(e["ms"], e["msg"]) for e in row.get("events", []) if e["dir"] == "out"]


def thread_of(msg):
    params = msg.get("params") or {}
    return params.get("threadId") or (params.get("thread") or {}).get("id")


def items(row, kind, phase="item/completed", thread=None):
    out = []
    for ms, msg in inbound(row):
        if msg.get("method") == phase and msg["params"]["item"].get("type") == kind:
            if thread is None or msg["params"].get("threadId") == thread:
                out.append((ms, msg["params"]["item"], msg["params"].get("threadId")))
    return out


def no_active_match(message: str) -> bool:
    lowered = (message or "").lower()
    return any(pattern in lowered for pattern in NO_ACTIVE_PATTERNS)


def err_of(response):
    if not isinstance(response, dict):
        return None
    return response.get("error")


def info_text(error) -> str:
    return json.dumps(error, ensure_ascii=False) if error else ""


def child_metrics(row):
    parent = row["thread_id"]
    out = {}
    spawns = [(ms, item) for ms, item, _ in items(row, "collabAgentToolCall") if item.get("tool") == "spawnAgent"]
    receivers = [r for _, item in spawns for r in (item.get("receiverThreadIds") or [])]
    child = receivers[0] if receivers else None
    out["spawn_attempted"] = bool(spawns) or bool(items(row, "collabAgentToolCall", "item/started"))
    out["child_thread_id"] = child
    out["wait_called"] = any(item.get("tool") == "wait" for _, item, _ in items(row, "collabAgentToolCall", "item/started"))
    out["child_event_count"] = sum(1 for _, msg in inbound(row) if child and thread_of(msg) == child)
    out["child_started_notification"] = any(msg.get("method") == "thread/started" and msg["params"]["thread"]["id"] == child for _, msg in inbound(row)) if child else None
    out["child_started_with_parent"] = any(msg.get("method") == "thread/started" and msg["params"]["thread"].get("parentThreadId") == parent
                                           for _, msg in inbound(row))
    completed = [(ms, msg) for ms, msg in inbound(row) if msg.get("method") == "turn/completed"]
    child_done = [(ms, msg["params"]["turn"].get("status")) for ms, msg in completed if child and thread_of(msg) == child]
    parent_done = [ms for ms, msg in completed if thread_of(msg) == parent]
    parent_statuses = [msg["params"]["turn"].get("status") for ms, msg in completed if thread_of(msg) == parent]
    out["parent_completed_turn_status"] = parent_statuses[-1] if parent_statuses else None
    out["child_turn_completed"] = bool(child_done)
    out["child_completed_status"] = child_done[-1][1] if child_done else None
    child_ms = child_done[-1][0] if child_done else None
    parent_ms = parent_done[-1] if parent_done else None
    out["child_completed_ms"] = child_ms
    out["parent_completed_ms"] = parent_ms
    out["parent_after_child"] = (parent_ms > child_ms) if (child_ms is not None and parent_ms is not None) else None
    spawn_started = [ms for ms, item, _ in items(row, "collabAgentToolCall", "item/started") if item.get("tool") == "spawnAgent"]
    spawn_done = [ms for ms, item in spawns]
    during = False
    if spawn_started and spawn_done:
        low, high = spawn_started[0], spawn_done[0]
        during = any(msg.get("method", "").endswith("requestApproval") and low <= ms <= high for ms, msg in inbound(row))
    out["approval_during_spawn"] = during
    out["child_command_approval_from_child"] = any(
        msg.get("method") == "item/commandExecution/requestApproval" and child and thread_of(msg) == child for _, msg in inbound(row))
    out["approval_count"] = sum(1 for _, msg in inbound(row) if msg.get("method", "").endswith("requestApproval"))
    facts = row["facts"]
    interrupts = [(ms, msg["params"]) for ms, msg in outbound(row) if msg.get("method") == "turn/interrupt"]
    parent_int = [ms for ms, params in interrupts if params.get("threadId") == parent]
    child_int = [ms for ms, params in interrupts if child and params.get("threadId") == child]
    out["parent_interrupt_ms"] = parent_int[0] if parent_int else None
    out["child_interrupt_ms"] = child_int[0] if child_int else None
    if parent_int:
        within = child_ms is not None and parent_int[0] < child_ms <= parent_int[0] + 12000 and not child_int
        out["child_done_within_12s_of_parent_interrupt"] = bool(within) if not child_int else bool(child_ms is not None and child_ms <= parent_int[0] + 12000)
    out["parent_turn_status"] = facts.get("parent_turn_status")
    out["sleep_alive_at_2s"] = facts.get("sleep_alive_at_2s")
    out["sleep_alive_at_12s"] = facts.get("sleep_alive_at_12s")
    out["sleep_alive_after_child_interrupt"] = facts.get("sleep_alive_after_child_interrupt")
    if child_int:
        after = [status for ms, status in child_done if ms >= child_int[0]]
        out["child_interrupt_effect"] = bool(after) and after[-1] == "interrupted"
    return out


def steer_metrics(row):
    facts = row["facts"]
    cond = row["condition"]
    out = {}

    def summarize(name, response):
        error = err_of(response)
        out[f"{name}_ok"] = bool(response and "result" in response)
        out[f"{name}_turn_id"] = ((response or {}).get("result") or {}).get("turnId")
        out[f"{name}_error"] = (error or {}).get("message") if error else None
        out[f"{name}_error_info"] = info_text(error) if error else None
        out[f"{name}_pattern_match"] = no_active_match((error or {}).get("message", "")) if error else None

    if cond == "steer_no_turn_fresh":
        summarize("steer", facts.get("steer_fresh_thread"))
    elif cond == "steer_accept":
        responses = facts.get("steer_responses") or {}
        summarize("wrong", responses.get("wrong"))
        summarize("valid", responses.get("valid"))
        summarize("empty", responses.get("empty"))
        summarize("after_completed", facts.get("steer_after_completed"))
        summarize("unknown_thread", facts.get("steer_unknown_thread"))
        out["active_turn_id"] = facts.get("steer_turn")
        out["valid_turn_matches"] = out["valid_turn_id"] is not None and out["valid_turn_id"] == facts.get("steer_turn")
        out["marker_effect"] = facts.get("marker_effect")
        valid_ms = None
        for ms, msg in inbound(row):
            if msg.get("id") == (facts.get("steer_ids") or {}).get("valid") and "method" not in msg:
                valid_ms = ms
        out["user_message_after_steer"] = any(
            msg.get("method") in ("item/started", "item/completed") and msg["params"]["item"].get("type") == "userMessage"
            and "steer-marker" in json.dumps(msg["params"]["item"]) and valid_ms is not None and ms >= valid_ms
            for ms, msg in inbound(row))
    elif cond == "steer_during_approval":
        summarize("steer", facts.get("steer_response"))
        out["first_effect"] = facts.get("first_effect")
        out["second_effect"] = facts.get("second_effect")
    elif cond == "steer_after_interrupt":
        summarize("steer", facts.get("steer_after_interrupt"))
        out["turn_status"] = facts.get("turn_status")
    elif cond in ("steer_review_turn", "steer_compact_turn"):
        summarize("steer", facts.get("steer_response"))
        text = info_text(err_of(facts.get("steer_response")))
        out["steer_not_steerable"] = "activeTurnNotSteerable" in text
        kind = re.search(r'"turnKind"\s*:\s*"(\w+)"', text)
        out["steer_turn_kind"] = kind.group(1) if kind else None
        start = facts.get("turn_start_response")
        out["turn_start_ok"] = bool(start and "result" in start)
        out["turn_start_error"] = (err_of(start) or {}).get("message")
        start_text = info_text(err_of(start))
        out["turn_start_not_steerable"] = "activeturnnotsteerable" in start_text.lower()
        start_kind = re.search(r"turn_kind: (\w+)", start_text)
        out["turn_start_turn_kind"] = start_kind.group(1).lower() if start_kind else None
        out["turn_start_error_code"] = (err_of(start) or {}).get("code")
        out["expected_turn_mismatch"] = (out.get("steer_error") or "").startswith("expected active turn id")
        out["turn_id_source"] = facts.get("turn_id_source")
        out["nonsteerable_turn_known"] = facts.get("nonsteerable_turn") not in (None, "unknown-turn") and not out["expected_turn_mismatch"]
        out["start_status"] = facts.get("start_status")
    return out


def approval_requests(row, method_prefix=None):
    return [(ms, msg) for ms, msg in inbound(row) if msg.get("method") and "id" in msg and (method_prefix is None or msg["method"] == method_prefix)]


def approval_metrics(row):
    facts = row["facts"]
    cond = row["condition"]
    out = {}
    if cond.startswith("mcp_"):
        requests = approval_requests(row, "mcpServer/elicitation/request")
        out["mcp_attempts"] = facts.get("mcp_tool_attempts")
        out["elicitation_requests"] = len(requests)
        meta_keys = sorted({key for _, msg in requests for key in ((msg["params"].get("_meta") or {}).keys())})
        out["request_meta_keys"] = ",".join(meta_keys)
        out["tool_name_in_meta"] = any(key in META_NAME_KEYS for key in meta_keys)
        names = [TOOL_RE.search(msg["params"].get("message", "")) for _, msg in requests]
        out["tool_name_in_message"] = bool(names) and all(match and match.group(1) == "write_like_tool" for match in names)
        out["server_name_field"] = bool(requests) and all(msg["params"].get("serverName") == "permission_fixture" for _, msg in requests)
        out["fixture_calls"] = len(facts.get("fixture_calls") or [])
        out["mcp_server_names"] = ",".join(sorted(str(item.get("name")) for item in facts.get("mcp_servers") or []))
        out["mcp_server_tool_counts"] = ",".join(f"{item.get('name')}:{item.get('tools')}" for item in facts.get("mcp_servers") or [])
    elif cond == "shell_always":
        requests = [msg for _, msg in approval_requests(row, "item/commandExecution/requestApproval") if "echo probe-always" in str(msg["params"].get("command"))]
        executions = [item for item in facts.get("command_results") or [] if "echo probe-always" in (item.get("command") or "") and item.get("status") == "completed"]
        out["command_requests"] = len(requests)
        out["executions"] = len(executions)
        out["available_decisions_first"] = json.dumps(requests[0]["params"].get("availableDecisions")) if requests else None
        out["accept_for_session_offered"] = ("acceptForSession" in (requests[0]["params"].get("availableDecisions") or [])) if requests else None
    elif cond == "edit_always":
        requests = approval_requests(row, "item/fileChange/requestApproval")
        out["file_requests"] = len(requests)
        out["file_changes"] = len(facts.get("file_changes") or [])
        out["file_request_param_keys"] = ",".join(sorted(requests[0][1]["params"].keys())) if requests else None
        out["s1"] = facts.get("s1")
        out["s2"] = facts.get("s2")
    return out


def sandbox_metrics(row):
    facts = row["facts"]
    results = facts.get("command_results") or []
    command_approvals = approval_requests(row, "item/commandExecution/requestApproval")
    out = {
        "command_approvals": len(command_approvals),
        "commands_run": len(results),
        "last_exit": results[-1]["exitCode"] if results else None,
        "last_status": results[-1]["status"] if results else None,
        "denial_in_output": any(bool(item.get("sandbox_denial")) or any(word in (item.get("output") or "").lower() for word in DENIAL) for item in results),
        "artifact_exists": facts.get("artifact_exists"),
        "outside_exists": facts.get("outside_exists"),
        "sub_exists": facts.get("sub_exists"),
        "fixture_error": any("believes it's in a workspace" in (item.get("output") or "") for item in results),
        "output_head": (results[-1].get("output") or "")[:160].replace("\n", " | ") if results else None,
        "approval_reasons": " | ".join(str(msg["params"].get("reason")) for _, msg in command_approvals if msg["params"].get("reason")),
        "approval_extra_permissions": any(msg["params"].get("additionalPermissions") for _, msg in command_approvals),
    }
    return out


def settings_metrics(row):
    facts = row["facts"]
    out = {}
    start = facts.get("start_settings") or {}
    sandbox = start.get("sandbox") or {}
    out["start_has_all"] = all(start.get(key) is not None for key in ("approvalPolicy", "approvalsReviewer", "cwd")) and "type" in sandbox and "networkAccess" in sandbox
    out["start_network_access"] = sandbox.get("networkAccess")
    plain = facts.get("resume_plain_settings") or {}
    explicit = facts.get("resume_settings") or {}
    for name, view in (("resume_plain", plain), ("resume_explicit", explicit)):
        sb = view.get("sandbox") or {}
        out[f"{name}_has_all"] = all(view.get(key) is not None for key in ("approvalPolicy", "approvalsReviewer", "cwd")) and "type" in sb and "networkAccess" in sb
        out[f"{name}_network_access"] = sb.get("networkAccess")
        out[f"{name}_policy"] = view.get("approvalPolicy")
        out[f"{name}_sandbox_type"] = sb.get("type")
    results = facts.get("command_results") or []
    last = results[-1] if results else {}
    output = (last.get("output") or "").strip()
    http_ok = bool(re.match(r"^[23]\d\d$", output[-3:])) if output else False
    out["curl_exit"] = last.get("exitCode")
    out["curl_output"] = output[-80:]
    out["curl_success"] = last.get("exitCode") == 0 and http_ok
    out["curl_failed"] = bool(results) and not out["curl_success"]
    out["command_approvals"] = len(approval_requests(row, "item/commandExecution/requestApproval"))
    updated = facts.get("settings_updated") or []
    out["settings_updated_n"] = len(updated)
    out["settings_updated_network_true"] = any(((m["params"].get("threadSettings") or {}).get("sandboxPolicy") or {}).get("networkAccess") is True for m in updated)
    out["settings_updated_keys"] = ",".join(sorted({key for m in updated for key in (m["params"].get("threadSettings") or {}).keys()}))
    config = facts.get("config_read") or {}
    out["config_read_empty"] = config.get("approval_policy") is None and config.get("sandbox_mode") is None
    turn_started = facts.get("turn_started") or []
    out["turn_started_keys"] = ",".join(sorted({key for turn in turn_started for key in turn.keys()}))
    return out


def auth_metrics(row):
    facts = row["facts"]
    return {"account_present": facts.get("account_present"), "account_type": facts.get("account_type"), "auth_is_symlink": facts.get("auth_is_symlink")}


def hook_metrics(row):
    facts = row["facts"]
    cond = row["condition"]
    if cond == "keychain_cleanup":
        return {"keychain_entry_removed": facts.get("keychain_entry_removed")}
    results = facts.get("command_results") or []
    log = facts.get("hook_log") or []
    events = facts.get("hook_events") or []
    out = {
        "hook_log_n": len(log),
        "hook_started_n": sum(1 for m in events if m.get("method") == "hook/started"),
        "hook_completed_n": sum(1 for m in events if m.get("method") == "hook/completed"),
        "hook_stdin_tools": ",".join(sorted({str((entry.get("stdin") or {}).get("tool_name")) for entry in log if isinstance(entry.get("stdin"), dict)})),
        "hook_stdin_keys": ",".join(sorted({key for entry in log if isinstance(entry.get("stdin"), dict) for key in entry["stdin"].keys()})),
        "hook_deny_output": any('"permissionDecision":"deny"' in (entry.get("stdout") or "") for entry in log),
        "hook_exit_codes": ",".join(str(entry.get("exit")) for entry in log),
        "hooks_list_trust": ",".join(str(item.get("trustStatus")) for item in facts.get("hooks_list") or []),
        "command_executed": any(item.get("status") == "completed" for item in results),
        "command_statuses": ",".join(str(item.get("status")) for item in results),
        "dummy_in_output": bool(facts.get("dummy_in_output")),
        "fake_key_in_output": any("fake-router-key-not-real" in (item.get("output") or "") for item in results),
        "edit_applied": facts.get("router_key_after") == "fake-edit",
        "approval_requests": len([1 for _, msg in inbound(row) if msg.get("method", "").endswith("requestApproval") and "id" in msg]),
        "hook_completed_text": " | ".join(entry.get("text", "") for m in events if m.get("method") == "hook/completed" for entry in (m["params"]["run"].get("entries") or []))[:200],
        "hook_completed_status": ",".join(str(m["params"]["run"].get("status")) for m in events if m.get("method") == "hook/completed"),
    }
    return out


METRICS = {
    "child": child_metrics, "steer": steer_metrics, "approval": approval_metrics, "sandbox": sandbox_metrics,
    "settings": settings_metrics, "auth": auth_metrics, "hook": hook_metrics,
}


def request_methods(row) -> list[str]:
    return sorted({msg["method"] for _, msg in inbound(row) if msg.get("method") and "id" in msg})


def main() -> int:
    rows = load_rows()
    processed = []
    for row in rows:
        base = {
            "run_id": row["run_id"], "trial_id": row["trial_id"], "condition": row["condition"], "topic": row["topic"],
            "ts_utc": row["ts_utc"], "status": row["status"], "model_calls": len(row.get("call_ordinals") or []),
            "request_methods": ",".join(request_methods(row)),
            "declined_commands": sum(1 for a in row.get("approvals", []) if a.get("method") == "item/commandExecution/requestApproval"
                                      and (a.get("response") or {}).get("decision") == "decline"),
        }
        if row["status"] == "driver_failure":
            base["error"] = row.get("error")
            processed.append(base)
            continue
        base.update(METRICS[row["topic"]](row))
        processed.append(base)
    OUT.mkdir(parents=True, exist_ok=True)
    columns = []
    for item in processed:
        for key in item:
            if key not in columns:
                columns.append(key)
    with (OUT / "trials.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.DictWriter(out, fieldnames=columns, lineterminator="\n")
        writer.writeheader()
        for item in processed:
            writer.writerow({key: ("" if item.get(key) is None else item.get(key)) for key in columns})
    with (OUT / "trials.jsonl").open("w", encoding="utf-8", newline="\n") as out:
        for item in processed:
            out.write(json.dumps(item, ensure_ascii=False, sort_keys=True) + "\n")
    print(f"processed {len(processed)} rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
