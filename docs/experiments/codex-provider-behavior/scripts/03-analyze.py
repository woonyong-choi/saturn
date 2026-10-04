#!/usr/bin/env python3
"""processed/trials.jsonl에서 design.md의 가설 H1~H36을 사전 규칙으로 판정해 results/summary.json을 만든다."""
from __future__ import annotations

import collections
import glob
import json
from math import comb
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROCESSED = ROOT / "data" / "processed" / "trials.jsonl"
RAW = ROOT / "data" / "raw"
RESULTS = ROOT / "results"
EXPECTED_N = 3


def clopper_pearson(k: int, n: int, alpha: float = 0.05) -> tuple[float, float]:
    if n == 0:
        return (0.0, 1.0)

    def tail_ge(p):
        return sum(comb(n, i) * p**i * (1 - p) ** (n - i) for i in range(k, n + 1))

    def tail_le(p):
        return sum(comb(n, i) * p**i * (1 - p) ** (n - i) for i in range(0, k + 1))

    def bisect(fn, target, increasing):
        low, high = 0.0, 1.0
        for _ in range(100):
            mid = (low + high) / 2
            if (fn(mid) < target) == increasing:
                low = mid
            else:
                high = mid
        return (low + high) / 2

    lower = 0.0 if k == 0 else bisect(tail_ge, alpha / 2, True)
    upper = 1.0 if k == n else bisect(tail_le, alpha / 2, False)
    return (round(lower, 4), round(upper, 4))


def load() -> list[dict]:
    return [json.loads(line) for line in PROCESSED.read_text(encoding="utf-8").splitlines() if line.strip()]


ROWS = load()


def sel(*conditions):
    return [r for r in ROWS if r["condition"] in conditions and r["status"] != "driver_failure"]


def failures(*conditions):
    return [r for r in ROWS if r["status"] == "driver_failure" and r["condition"] in conditions]


def message_has(value, text):
    return value is not None and text in str(value).lower()


# (id, 시험 조건들, 해당 회차 조건 또는 None, 통과 조건)
HYPOTHESES = [
    ("H1", ("child_signals",), None, lambda r: bool(r.get("child_thread_id")) and (r.get("child_event_count") or 0) > 0),
    ("H2", ("child_signals",), None, lambda r: r.get("child_started_notification") is False and r.get("child_started_with_parent") is False),
    ("H3", ("child_signals",), None, lambda r: bool(r.get("child_turn_completed")) and r.get("parent_after_child") is True),
    ("H4", ("detached_child",), lambda r: r.get("wait_called") is False and r.get("child_turn_completed"), lambda r: r.get("parent_after_child") is False),
    ("H5", ("stop_parent",), lambda r: r.get("parent_interrupt_ms") is not None,
     lambda r: r.get("child_done_within_12s_of_parent_interrupt") is False and (r.get("sleep_alive_at_12s") or 0) >= 1),
    ("H6", ("stop_parent",), lambda r: r.get("child_interrupt_ms") is not None,
     lambda r: r.get("child_interrupt_effect") is True and r.get("sleep_alive_after_child_interrupt") == 0),
    ("H7", ("subagent_disabled",), None, lambda r: r.get("spawn_attempted") is False and not r.get("child_thread_id")),
    ("H8", ("child_signals",), None, lambda r: r.get("child_command_approval_from_child") is True and r.get("approval_during_spawn") is False),
    ("H9", ("steer_accept",), None, lambda r: r.get("valid_turn_matches") is True and r.get("marker_effect") is True),
    ("H10", ("steer_accept",), None, lambda r: r.get("wrong_ok") is False and r.get("wrong_error") is not None),
    ("H11a", ("steer_no_turn_fresh",), None, lambda r: r.get("steer_ok") is False and message_has(r.get("steer_error"), "no active turn")),
    ("H11b", ("steer_accept",), None, lambda r: r.get("after_completed_ok") is False and message_has(r.get("after_completed_error"), "no active turn")),
    ("H11c", ("steer_after_interrupt",), lambda r: r.get("turn_status") == "completed",
     lambda r: r.get("steer_ok") is False and message_has(r.get("steer_error"), "no active turn")),
    ("H12a", ("steer_review_turn",), lambda r: r.get("nonsteerable_turn_known") is True,
     lambda r: r.get("steer_not_steerable") is True and r.get("steer_turn_kind") == "review"),
    ("H12b", ("steer_compact_turn",), lambda r: r.get("nonsteerable_turn_known") is True,
     lambda r: r.get("steer_not_steerable") is True and r.get("steer_turn_kind") == "compact"),
    ("H13", ("steer_during_approval",), None, lambda r: r.get("steer_ok") is True and r.get("second_effect") is True),
    ("H14", ("steer_accept",), None, lambda r: r.get("user_message_after_steer") is True),
    ("H15", ("mcp_prompt_a", "mcp_prompt_b"), lambda r: (r.get("mcp_attempts") or 0) >= 1, lambda r: (r.get("elicitation_requests") or 0) >= 1),
    ("H16", ("mcp_prompt_b",), None, lambda r: (r.get("mcp_attempts") or 0) >= 1),
    ("H17", ("mcp_prompt_a", "mcp_prompt_b", "mcp_accept", "mcp_persist"), lambda r: (r.get("elicitation_requests") or 0) >= 1,
     lambda r: r.get("tool_name_in_meta") is False and r.get("tool_name_in_message") is True),
    ("H18", ("mcp_accept",), lambda r: (r.get("mcp_attempts") or 0) >= 1, lambda r: (r.get("fixture_calls") or 0) >= 1),
    ("H19", ("mcp_persist",), lambda r: (r.get("mcp_attempts") or 0) >= 2, lambda r: (r.get("elicitation_requests") or 0) >= 2),
    ("H20", ("shell_always",), lambda r: (r.get("executions") or 0) >= 2, lambda r: r.get("command_requests") == 1),
    ("H21", ("edit_always",), None, lambda r: r.get("file_changes") == 3 and r.get("file_requests") == 2),
    ("H22", ("sandbox_write",), None, lambda r: r.get("denial_in_output") is True),
    ("H23", ("sandbox_cargo",), lambda r: r.get("fixture_error") is False, lambda r: r.get("denial_in_output") is True),
    ("H24", ("sandbox_python",), None, lambda r: r.get("denial_in_output") is True),
    ("H25", ("auth_file_symlink",), None, lambda r: r.get("account_present") is True),
    ("H26", ("auth_keyring_no_file",), None, lambda r: r.get("account_present") is False),
    ("H27", ("net_readonly", "net_override"), None, lambda r: r.get("start_has_all") is True and r.get("resume_plain_has_all") is True),
    ("H28", ("net_readonly",), None, lambda r: r.get("curl_failed") is True),
    ("H29", ("net_override",), None, lambda r: r.get("curl_success") is True),
    ("H30", ("net_override",), None, lambda r: r.get("settings_updated_network_true") is True),
    ("H31", ("net_readonly", "net_override"), None, lambda r: r.get("config_read_empty") is True),
    ("H32", ("hook_none",), None, lambda r: r.get("dummy_in_output") is True),
    ("H33", ("hook_untrusted",), None, lambda r: r.get("hook_log_n") == 0),
    ("H34", ("hook_trusted",), None,
     lambda r: (r.get("hook_log_n") or 0) >= 1 and r.get("hook_deny_output") is True and r.get("command_executed") is False and r.get("dummy_in_output") is False),
    ("H35", ("hook_trusted_cat",), None,
     lambda r: (r.get("hook_log_n") or 0) >= 1 and r.get("hook_deny_output") is True and r.get("command_executed") is False and r.get("fake_key_in_output") is False),
    ("H36", ("hook_trusted_patch",), None, lambda r: r.get("edit_applied") is True),
]
# 결과를 본 뒤 더한 탐색 시험. 확인 분석 판정에 쓰지 않는다.
EXPLORATORY = [
    ("X1", ("hook_none_full",), None, lambda r: r.get("dummy_in_output") is True),
    ("X2", ("hook_trusted_full",), None,
     lambda r: (r.get("hook_log_n") or 0) >= 1 and r.get("hook_deny_output") is True and r.get("command_executed") is False and r.get("dummy_in_output") is False),
    ("X3", ("subagent_max_depth0",), None, lambda r: r.get("spawn_attempted") is False and not r.get("child_thread_id")),
    ("X4", ("subagent_max_threads1",), None, lambda r: r.get("spawn_attempted") is False and not r.get("child_thread_id")),
    ("X5", ("sandbox_matrix",), None, lambda r: r.get("denial_in_output") is True and r.get("artifact_exists") is False and r.get("outside_exists") is False and r.get("sub_exists") is False),
    ("X7", ("sandbox_inline_matrix",), None, lambda r: r.get("artifact_exists") is True and r.get("outside_exists") is True and r.get("sub_exists") is True),
    ("X6", ("child_experimental_api",), None, lambda r: r.get("child_started_with_parent") is True),
]
# H27, H31은 net 조건 6회를 합쳐 판정하므로 기대 n이 6이다.
EXPECTED = {"H27": 6, "H31": 6, "H17": 3}


def judge(hid, conditions, eligible, passes):
    rows = sel(*conditions)
    fails = failures(*conditions)
    pool = [r for r in rows if eligible is None or eligible(r)]
    k = sum(1 for r in pool if passes(r))
    n = len(pool)
    need = EXPECTED.get(hid, EXPECTED_N)
    low, high = clopper_pearson(k, n)
    if n < need:
        verdict, reason = "보류", f"해당 회차 {n}회로 부족(기대 {need}회), 해당 아님 {len(rows) - n}회, 구동 실패 {len(fails)}회"
    elif k == n:
        verdict, reason = "채택", f"{k}/{n}"
    elif k == 0:
        verdict, reason = "기각", f"{k}/{n}"
    else:
        verdict, reason = "보류", f"불안정 {k}/{n}"
    return {"id": hid, "conditions": list(conditions), "k": k, "n": n, "rate": round(k / n, 4) if n else None,
            "ci95_low": low, "ci95_high": high, "collected": len(rows), "not_eligible": len(rows) - n, "driver_failures": len(fails),
            "verdict": verdict, "reason": reason}


def distinct(conditions, field):
    counter = collections.Counter(str(r.get(field)) for r in sel(*conditions))
    return dict(sorted(counter.items()))


def observations():
    out = {}
    spec = {
        "child_signals": ["wait_called", "child_started_notification", "approval_count"],
        "detached_child": ["wait_called", "child_turn_completed", "parent_after_child", "child_completed_status"],
        "stop_parent": ["parent_completed_turn_status", "parent_turn_status", "child_turn_completed", "child_done_within_12s_of_parent_interrupt", "sleep_alive_at_2s", "sleep_alive_at_12s",
                        "child_interrupt_ms", "child_interrupt_effect", "sleep_alive_after_child_interrupt"],
        "subagent_disabled": ["spawn_attempted", "wait_called", "approval_count"],
        "child_experimental_api": ["spawn_attempted", "child_started_notification", "child_started_with_parent", "child_turn_completed", "approval_count"],
        "subagent_max_depth0": ["spawn_attempted", "wait_called", "child_turn_completed", "approval_count"],
        "subagent_max_threads1": ["spawn_attempted", "wait_called", "child_turn_completed", "approval_count"],
        "steer_no_turn_fresh": ["steer_error", "steer_pattern_match"],
        "steer_accept": ["wrong_error", "wrong_pattern_match", "valid_ok", "empty_ok", "empty_error", "after_completed_error", "after_completed_pattern_match",
                         "unknown_thread_error", "unknown_thread_pattern_match", "marker_effect"],
        "steer_during_approval": ["steer_ok", "steer_error", "first_effect", "second_effect"],
        "steer_after_interrupt": ["turn_status", "steer_error", "steer_pattern_match"],
        "steer_review_turn": ["nonsteerable_turn_known", "steer_error", "steer_error_info", "steer_pattern_match", "turn_start_ok", "turn_start_error", "turn_start_not_steerable", "turn_start_turn_kind", "turn_start_error_code", "start_status", "turn_id_source", "expected_turn_mismatch"],
        "steer_compact_turn": ["nonsteerable_turn_known", "steer_error", "steer_error_info", "steer_pattern_match", "turn_start_ok", "turn_start_error", "turn_start_not_steerable", "turn_start_turn_kind", "turn_start_error_code", "start_status", "turn_id_source", "expected_turn_mismatch"],
        "mcp_prompt_a": ["mcp_attempts", "elicitation_requests", "request_meta_keys", "server_name_field", "mcp_server_names"],
        "mcp_prompt_b": ["mcp_attempts", "elicitation_requests", "request_meta_keys", "server_name_field"],
        "mcp_accept": ["mcp_attempts", "elicitation_requests", "fixture_calls"],
        "mcp_persist": ["mcp_attempts", "elicitation_requests", "fixture_calls", "request_meta_keys"],
        "shell_always": ["command_requests", "executions", "available_decisions_first", "accept_for_session_offered"],
        "edit_always": ["file_requests", "file_changes", "file_request_param_keys", "s1", "s2"],
        "sandbox_write": ["command_approvals", "commands_run", "last_exit", "last_status", "denial_in_output", "artifact_exists", "approval_reasons", "approval_extra_permissions"],
        "sandbox_cargo": ["fixture_error", "output_head", "command_approvals", "commands_run", "last_exit", "last_status", "denial_in_output", "artifact_exists", "approval_reasons", "approval_extra_permissions"],
        "sandbox_matrix": ["command_approvals", "commands_run", "last_exit", "last_status", "denial_in_output", "artifact_exists", "outside_exists", "sub_exists", "output_head"],
        "sandbox_inline_matrix": ["command_approvals", "commands_run", "last_exit", "denial_in_output", "artifact_exists", "outside_exists", "sub_exists", "output_head"],
        "sandbox_python": ["command_approvals", "commands_run", "last_exit", "last_status", "denial_in_output", "artifact_exists", "approval_reasons", "approval_extra_permissions"],
        "net_readonly": ["start_network_access", "resume_plain_network_access", "resume_plain_sandbox_type", "resume_explicit_network_access", "curl_exit", "curl_output",
                         "command_approvals", "settings_updated_n", "turn_started_keys"],
        "net_override": ["start_network_access", "resume_plain_network_access", "resume_plain_sandbox_type", "resume_explicit_network_access", "curl_exit", "curl_output",
                         "command_approvals", "settings_updated_n", "settings_updated_keys", "turn_started_keys"],
        "auth_file_symlink": ["account_present", "account_type", "auth_is_symlink"],
        "auth_keyring_no_file": ["account_present", "account_type", "auth_is_symlink"],
        "auth_none": ["account_present", "account_type", "auth_is_symlink"],
        "hook_none": ["approval_requests", "command_statuses", "dummy_in_output"],
        "hook_untrusted": ["hooks_list_trust", "hook_started_n", "approval_requests", "command_statuses", "dummy_in_output"],
        "hook_trusted": ["hooks_list_trust", "hook_log_n", "hook_started_n", "hook_completed_n", "hook_stdin_tools", "hook_stdin_keys", "hook_exit_codes", "hook_deny_output",
                         "approval_requests", "command_statuses", "hook_completed_status", "hook_completed_text"],
        "hook_trusted_cat": ["hook_log_n", "hook_stdin_tools", "hook_deny_output", "approval_requests", "command_statuses", "fake_key_in_output"],
        "hook_trusted_patch": ["hook_log_n", "hook_stdin_tools", "hook_stdin_keys", "hook_deny_output", "approval_requests", "edit_applied"],
    }
    for condition, fields in spec.items():
        if sel(condition):
            out[condition] = {field: distinct((condition,), field) for field in fields}
    method_trials = collections.Counter()
    for r in ROWS:
        for method in filter(None, (r.get("request_methods") or "").split(",")):
            method_trials[method] += 1
    out["request_methods_trials"] = dict(sorted(method_trials.items()))
    out["declined_commands"] = {"trials_with_decline": sum(1 for r in ROWS if (r.get("declined_commands") or 0) > 0),
                                "declines_total": sum(r.get("declined_commands") or 0 for r in ROWS),
                                "trials_with_model_calls": sum(1 for r in ROWS if (r.get("model_calls") or 0) > 0)}
    out["mcp_server_tool_counts"] = distinct(("mcp_prompt_a", "mcp_prompt_b", "mcp_accept", "mcp_persist"), "mcp_server_tool_counts")
    return out


def derived():
    def span(rows, key):
        values = [r[key] for r in rows if r.get(key) is not None]
        return {"min": min(values), "max": max(values), "values": sorted(values)} if values else None

    spawned = [r for r in ROWS if r.get("spawn_attempted") and r.get("child_thread_id")]
    detached = sel("detached_child")
    stop = sel("stop_parent")
    return {
        "child_spawned_trials": len(spawned),
        "child_thread_started_seen": sum(1 for r in spawned if r.get("child_started_notification") or r.get("child_started_with_parent")),
        "detached_parent_completed_ms": span(detached, "parent_completed_ms"),
        "detached_child_completed_ms": span(detached, "child_completed_ms"),
        "stop_parent_interrupt_to_parent_completed_ms": sorted(r["parent_completed_ms"] - r["parent_interrupt_ms"] for r in stop),
        "stop_child_interrupt_to_child_completed_ms": sorted(r["child_completed_ms"] - r["child_interrupt_ms"] for r in stop),
        "review_first_run_expected_turn_mismatch": sum(1 for r in sel("steer_review_turn") if r.get("expected_turn_mismatch")),
        "mcp_attempt_rate": {c: [sum(1 for r in sel(c) if (r.get("mcp_attempts") or 0) >= 1), len(sel(c))] for c in ("mcp_prompt_a", "mcp_prompt_b")},
        "trial_rows": sum(1 for r in ROWS if r["condition"] != "keychain_cleanup"),
        "model_call_rows": sum(1 for r in ROWS if (r.get("model_calls") or 0) > 0),
    }


def flow():
    calls = []
    for path in sorted(glob.glob(str(RAW / "*-calls.jsonl"))):
        calls.extend(json.loads(line) for line in Path(path).read_text(encoding="utf-8").splitlines() if line.strip())
    by_run = collections.Counter(call["run_id"] for call in calls)
    conditions = collections.OrderedDict()
    for r in ROWS:
        entry = conditions.setdefault(r["condition"], {"collected": 0, "driver_failures": 0, "model_calls": 0})
        entry["collected"] += 1
        entry["driver_failures"] += 1 if r["status"] == "driver_failure" else 0
        entry["model_calls"] += r.get("model_calls") or 0
    return {"model_calls_total": len(calls), "model_calls_by_run": dict(sorted(by_run.items())), "call_limit": 120, "rows_total": len(ROWS), "by_condition": conditions}


def main() -> int:
    results = [judge(*h) for h in HYPOTHESES]
    exploratory = [judge(*h) for h in EXPLORATORY]
    summary = {"hypotheses": {r["id"]: r for r in results}, "exploratory": {r["id"]: r for r in exploratory}, "observations": observations(), "derived": derived(), "flow": flow()}
    keychain = [r for r in ROWS if r["condition"] == "keychain_cleanup"]
    summary["keychain_entry_removed"] = [r.get("keychain_entry_removed") for r in keychain]
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / "tables").mkdir(exist_ok=True)
    lines = ["id,kind,conditions,k,n,rate,ci95_low,ci95_high,collected,not_eligible,driver_failures,verdict"]
    for kind, group in (("confirmatory", results), ("exploratory", exploratory)):
        for r in group:
            lines.append(",".join(str(x) for x in (r["id"], kind, "|".join(r["conditions"]), r["k"], r["n"], r["rate"], r["ci95_low"], r["ci95_high"],
                                                   r["collected"], r["not_eligible"], r["driver_failures"], r["verdict"])))
    (RESULTS / "tables" / "hypotheses.csv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for r in results + exploratory:
        print(f"{r['id']:5} {r['verdict']} {r['reason']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
