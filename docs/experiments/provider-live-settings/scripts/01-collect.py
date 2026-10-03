#!/usr/bin/env python3
from __future__ import annotations

import datetime as dt
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from common import (CLAUDE_MODEL, CODEX_MODEL, ROOT, WORKTREE, ClaudeDriver, CodexDriver,
                    marker_path, marker_prompt, private_log, redact, setup_codex_home, utc_now)

RAW = ROOT / "data" / "raw"
CAP = {"claude": 40, "codex": 40}
COUNTS = {"claude": 0, "codex": 0}


def command(command: list[str], timeout: int = 60):
    try:
        result = subprocess.run(command, cwd=WORKTREE, capture_output=True, text=True,
                                timeout=timeout, check=False)
        return result.returncode, redact(result.stdout), redact(result.stderr)
    except (OSError, subprocess.TimeoutExpired) as error:
        return 127, "", redact(str(error))


def versions() -> dict:
    result = {}
    for name in ("claude", "codex"):
        code, stdout, stderr = command([name, "--version"])
        result[name] = {"exit_code": code, "stdout": stdout, "stderr": stderr}
    return result


def private_text(name: str, text: str) -> str:
    return private_log(name, [{"kind": "text", "text": text}])


def row(run_id: str, trial_id: str, condition: str, provider: str, result: str,
        process_id: str = "none", decision: str = "not_requested", effect=False,
        next_effect=False, restart_effect=False, log="", **extra):
    value = {"run_id": run_id, "trial_id": trial_id, "condition": condition,
             "ts_utc": utc_now(), "provider": provider, "request_result": result,
             "process_id": process_id, "tool_decision": decision,
             "fixture_effect": bool(effect), "next_turn_effect": bool(next_effect),
             "restart_effect": bool(restart_effect), "private_log": log}
    value.update(extra)
    return value


def count(provider: str, amount: int = 1) -> bool:
    COUNTS[provider] += amount
    return COUNTS[provider] <= CAP[provider]


def inventory(run_id: str) -> list[dict]:
    rows = []
    for name in ("claude", "codex"):
        code, stdout, stderr = command([name, "--help"])
        rows.append(row(run_id, f"inventory-{name}-help", f"inventory.{name}.help", name,
                        "observed" if code == 0 else "error", log=private_text(
                            f"{run_id}-inventory-{name}-help.txt", f"exit={code}\n{stdout}\n{stderr}")))
        code, stdout, stderr = command([name, "--version"])
        rows.append(row(run_id, f"inventory-{name}-version", f"inventory.{name}.version", name,
                        "observed" if code == 0 else "error", log=private_text(
                            f"{run_id}-inventory-{name}-version.txt", f"exit={code}\n{stdout}\n{stderr}")))
    schema_dir = WORKTREE / ".runtime" / f"schema-{run_id}"
    schema_dir.mkdir(parents=True, exist_ok=True)
    code, stdout, stderr = command(["codex", "app-server", "generate-json-schema", "--out", str(schema_dir)], 90)
    files = sorted(str(path.relative_to(schema_dir)) for path in schema_dir.rglob("*") if path.is_file())
    schema_text = "\n".join(files) + f"\nexit={code}\nstdout={stdout}\nstderr={stderr}\n"
    rows.append(row(run_id, "inventory-codex-schema", "inventory.codex.schema", "codex",
                    "observed" if code == 0 else "error", log=private_text(
                        f"{run_id}-inventory-codex-schema.txt", schema_text),
                    schema_files=files, schema_sha256=hashlib.sha256(schema_text.encode()).hexdigest()))
    return rows


def claude_permission_mode(run_id: str) -> list[dict]:
    rows = []
    for trial in range(1, 4):
        if not count("claude", 2):
            break
        trial_id = f"claude-permission-mode-{trial}"
        before = marker_path(trial_id, "before")
        after = marker_path(trial_id, "after")
        driver = ClaudeDriver(f"00000000-0000-4000-8000-{trial:012d}", "manual")
        driver.user(marker_prompt(before, f"CLAUDE_BEFORE_{trial}"))
        before_decisions, before_result = driver.finish_turn(permission="deny")
        control = driver.control(f"live-settings-{trial}", {"subtype": "set_permission_mode", "mode": "dontAsk"})
        driver.user(marker_prompt(after, f"CLAUDE_AFTER_{trial}"))
        decisions, result = driver.finish_turn(permission="allow")
        stderr = driver.close()
        effect = after.exists() and not before.exists()
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr,
            "before_marker": str(before), "before_marker_exists": before.exists(),
            "after_marker": str(after), "after_marker_exists": after.exists()}])
        response = control.get("response", {}) if control else {}
        rows.append(row(run_id, trial_id, "claude.set_permission_mode", "claude",
                        "success" if response.get("subtype") == "success" else "error_or_timeout",
                        str(driver.pid), decisions[-1]["behavior"] if decisions else "not_requested",
                        effect, effect, False, log, control_subtype=response.get("subtype"),
                        before_decision=before_decisions[-1]["behavior"] if before_decisions else "not_requested",
                        before_result_seen=bool(before_result), result_seen=bool(result),
                        mode_requested="dontAsk"))
    return rows


def claude_unsupported_control(run_id: str) -> list[dict]:
    candidates = ["set_model", "set_max_thinking_tokens", "mcp_message", "rewind_files", "stop_task"]
    rows = []
    for candidate in candidates:
        for trial in range(1, 4):
            trial_id = f"claude-control-{candidate}-{trial}"
            driver = ClaudeDriver(f"10000000-0000-4000-8000-{trial:011d}{len(candidate):01d}", "plan")
            init = driver.init(1)
            params = {"subtype": candidate}
            if candidate == "set_model":
                params["model"] = CLAUDE_MODEL
            if candidate == "set_max_thinking_tokens":
                params["max_thinking_tokens"] = 1000
            control = driver.control(f"live-unsupported-{candidate}-{trial}", params, timeout=5.0)
            stderr = driver.close()
            response = control.get("response", {}) if control else {}
            log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr}])
            rows.append(row(run_id, trial_id, f"claude.control.{candidate}", "claude",
                            "success" if response.get("subtype") == "success" else "unsupported_or_error",
                            str(driver.pid), log=log, control_subtype=response.get("subtype"),
                            init_seen=bool(init), response=response))
    return rows


def claude_ask_user(run_id: str) -> list[dict]:
    rows = []
    for trial in range(1, 4):
        if not count("claude", 2):
            break
        trial_id = f"claude-ask-user-{trial}"
        driver = ClaudeDriver(f"20000000-0000-4000-8000-{trial:012d}", "plan")
        init = driver.init(3)
        driver.user("Use AskUserQuestion to ask me one short question, then wait.")
        denied, first_result = driver.finish_turn(ask_user="deny", permission="allow", timeout=45)
        driver.user("Use AskUserQuestion once more and wait for the answer.")
        allowed, second_result = driver.finish_turn(ask_user="allow", permission="allow", timeout=45)
        stderr = driver.close()
        ask_decisions = [*denied, *allowed]
        positions = [index for index, value in enumerate(ask_decisions) if value["tool"] == "AskUserQuestion"]
        first = ask_decisions[positions[0]] if positions else None
        second = ask_decisions[positions[1]] if len(positions) > 1 else None
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr}])
        rows.append(row(run_id, trial_id, "claude.AskUserQuestion.deny_then_allow", "claude",
                        "success" if first and second else "not_observed", str(driver.pid),
                        f"{first['behavior'] if first else 'none'}->{second['behavior'] if second else 'none'}",
                        bool(second), bool(second), False, log, init_seen=bool(init),
                        first_result=bool(first_result), second_result=bool(second_result)))
    return rows


def codex_config_batch(run_id: str) -> list[dict]:
    rows = []
    for trial in range(1, 4):
        if not count("codex", 2):
            break
        trial_id = f"codex-config-batch-{trial}"
        home = setup_codex_home(trial_id, 'approval_policy = "on-request"\nsandbox_mode = "workspace-write"\n', "")
        driver = CodexDriver(home)
        driver.initialize()
        started = driver.start_thread()
        thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
        before_marker = marker_path(trial_id, "batch-before")
        after_marker = marker_path(trial_id, "batch-after")
        before, before_events = driver.turn(thread_id, marker_prompt(before_marker, f"CODEX_BATCH_{trial}")) if thread_id else (None, [])
        write = driver.request("config/batchWrite", {"filePath": str(home / "config.toml"),
            "edits": [{"keyPath": "approval_policy", "mergeStrategy": "replace", "value": "never"}],
            "reloadUserConfig": True})
        read_after = driver.request("config/read", {"cwd": str(home), "includeLayers": True})
        after, after_events = driver.turn(thread_id, marker_prompt(after_marker, f"CODEX_BATCH_AFTER_{trial}")) if thread_id else (None, [])
        stderr = driver.close()
        approval_count = len(driver.server_requests)
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr,
            "batch_write": write, "config_read_after": read_after, "before_events": before_events, "after_events": after_events,
            "before_marker_exists": before_marker.exists(), "after_marker_exists": after_marker.exists()}])
        rows.append(row(run_id, trial_id, "codex.config.batchWrite.reloadUserConfig", "codex",
                        "success" if write and "result" in write else "error_or_timeout", str(driver.pid),
                        "approval" if approval_count else "not_requested", after_marker.exists(), after_marker.exists(), False, log,
                        reload_user_config=True, approval_requests=approval_count, thread_id=thread_id,
                        config_after=read_after.get("result", {}).get("config", {}).get("approval_policy") if read_after else None))
    return rows


def codex_feature_reload(run_id: str) -> list[dict]:
    rows = []
    for trial in range(1, 4):
        trial_id = f"codex-feature-reload-{trial}"
        home = setup_codex_home(trial_id, 'approval_policy = "never"\nsandbox_mode = "workspace-write"\n', "")
        driver = CodexDriver(home)
        driver.initialize()
        started = driver.start_thread()
        thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
        before = driver.request("experimentalFeature/list", {"threadId": thread_id})
        changed = driver.request("experimentalFeature/enablement/set", {
            "enablement": {"tool_suggest": False}})
        after = driver.request("experimentalFeature/list", {"threadId": thread_id})
        mcp_reload = driver.request("config/mcpServer/reload", None)
        stderr = driver.close()
        def feature_value(response):
            for feature in (response or {}).get("result", {}).get("data", []):
                if feature.get("name") == "tool_suggest":
                    return feature.get("enabled")
            return None
        effect = feature_value(after) is False
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr,
            "before": before, "changed": changed, "after": after, "mcp_reload": mcp_reload}])
        rows.append(row(run_id, trial_id, "codex.experimentalFeature.enablement.set", "codex",
                        "success" if changed and "result" in changed else "error_or_timeout",
                        str(driver.pid), log=log, effect=effect, next_effect=effect,
                        feature="tool_suggest", feature_after=feature_value(after),
                        mcp_reload_result=(mcp_reload or {}).get("result")))
    return rows


def codex_turn_override(run_id: str) -> list[dict]:
    rows = []
    for route, override in [("approvalPolicy", {"approvalPolicy": "never"}),
                            ("sandboxPolicy", {"sandboxPolicy": {"type": "readOnly"}})]:
        for trial in range(1, 4):
            if not count("codex", 2):
                return rows
            trial_id = f"codex-turn-{route}-{trial}"
            home = setup_codex_home(trial_id, 'approval_policy = "on-request"\nsandbox_mode = "workspace-write"\n', "")
            driver = CodexDriver(home)
            driver.initialize()
            started = driver.start_thread(approval_policy="on-request", sandbox="workspace-write")
            thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
            first_marker = marker_path(trial_id, "override-first")
            next_marker = marker_path(trial_id, "override-next")
            turn_result, events = driver.turn(thread_id, marker_prompt(first_marker, f"CODEX_{route}_{trial}"), override) if thread_id else (None, [])
            next_result, next_events = driver.turn(thread_id, marker_prompt(next_marker, f"CODEX_{route}_NEXT_{trial}")) if thread_id else (None, [])
            stderr = driver.close()
            log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr, "events": events, "next_events": next_events}])
            rows.append(row(run_id, trial_id, f"codex.turn.start.{route}", "codex",
                            "success" if turn_result and "result" in turn_result else "error_or_timeout", str(driver.pid),
                            "approval" if driver.server_requests else "not_requested", next_marker.exists(), next_marker.exists(), False, log,
                            override=override, thread_id=thread_id, next_turn_response=bool(next_result)))
    return rows


def codex_file_changes(run_id: str) -> list[dict]:
    rows = []
    for route in ("config.toml", "rules/default.rules"):
        for trial in range(1, 4):
            if not count("codex", 3):
                return rows
            trial_id = f"codex-file-{route.replace('/', '-')}-{trial}"
            home = setup_codex_home(trial_id, 'approval_policy = "never"\nsandbox_mode = "workspace-write"\n',
                                    'prefix_rule(pattern = ["printf"], decision = "allow")\n')
            driver = CodexDriver(home)
            driver.initialize()
            started = driver.start_thread()
            thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
            first_marker = marker_path(trial_id, "file-before")
            next_marker = marker_path(trial_id, "file-next")
            first, first_events = driver.turn(thread_id, marker_prompt(first_marker, f"CODEX_FILE_BASE_{trial}")) if thread_id else (None, [])
            if route == "config.toml":
                (home / "config.toml").write_text('approval_policy = "on-request"\nsandbox_mode = "workspace-write"\n', encoding="utf-8")
            else:
                (home / "rules" / "default.rules").write_text('prefix_rule(pattern = ["printf"], decision = "forbidden")\n', encoding="utf-8")
            second, second_events = driver.turn(thread_id, marker_prompt(next_marker, f"CODEX_FILE_NEXT_{trial}")) if thread_id else (None, [])
            same_effect = next_marker.exists()
            driver.close()
            fresh = CodexDriver(home)
            fresh.initialize()
            fresh_started = fresh.start_thread()
            fresh_id = (fresh_started or {}).get("result", {}).get("thread", {}).get("id")
            restart_marker = marker_path(trial_id, "restart")
            third, third_events = fresh.turn(fresh_id, marker_prompt(restart_marker, f"CODEX_FILE_RESTART_{trial}")) if fresh_id else (None, [])
            restart_effect = restart_marker.exists()
            stderr = fresh.close()
            log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + fresh.events + [{"stderr": stderr,
                "first_events": first_events, "second_events": second_events, "third_events": third_events,
                "before_marker_exists": first_marker.exists(), "next_marker_exists": next_marker.exists(),
                "restart_marker_exists": restart_effect}])
            rows.append(row(run_id, trial_id, f"codex.file.{route}", "codex", "observed", str(driver.pid),
                            "approval" if fresh.server_requests else "not_requested", same_effect, same_effect,
                            restart_effect != same_effect, log, same_process_effect=same_effect,
                            restart_process_effect=restart_effect, thread_id=thread_id,
                            fixture_distinguishes=False))
    return rows


def main() -> int:
    commit = subprocess.check_output(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE, text=True).strip()
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + f"-{commit}"
    selected = set(sys.argv[1:]) or {"inventory", "claude", "codex"}
    rows = inventory(run_id) if "inventory" in selected else []
    if "claude" in selected or "claude-permission" in selected:
        rows.extend(claude_permission_mode(run_id))
    if "claude" in selected or "claude-controls" in selected:
        rows.extend(claude_unsupported_control(run_id))
    if "claude" in selected or "claude-ask" in selected:
        rows.extend(claude_ask_user(run_id))
    if "codex" in selected or "codex-batch" in selected:
        rows.extend(codex_config_batch(run_id))
    if "codex" in selected or "codex-feature" in selected:
        rows.extend(codex_feature_reload(run_id))
    if "codex" in selected or "codex-turn" in selected:
        rows.extend(codex_turn_override(run_id))
    if "codex" in selected or "codex-files" in selected:
        rows.extend(codex_file_changes(run_id))
    RAW.mkdir(parents=True, exist_ok=True)
    path = RAW / f"{run_id}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as stream:
        for value in rows:
            stream.write(json.dumps(redact(value), ensure_ascii=False, separators=(",", ":")) + "\n")
    env = {"run_id": run_id, "date_utc": utc_now(), "commit": commit, "python": sys.version.split()[0],
           "versions": versions(), "models": {"claude": CLAUDE_MODEL, "codex": CODEX_MODEL},
           "model_call_caps": CAP, "model_calls_observed": COUNTS}
    (ROOT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"raw": str(path), "model_calls": COUNTS}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
