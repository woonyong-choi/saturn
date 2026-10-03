#!/usr/bin/env python3
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

from common import (CODEX_MODEL, MAX_TURN_CALLS, ROOT, WORKTREE, CodexDriver,
                    marker_path, private_log, question_prompt, redact, response_status,
                    setup_codex_home, touch_prompt, utc_now)

RAW = ROOT / "data" / "raw"
TURN_CALLS = 0
STOP = False


def command(args: list[str], timeout: int = 90):
    try:
        result = subprocess.run(args, cwd=WORKTREE, capture_output=True, text=True,
                                timeout=timeout, check=False)
        return result.returncode, redact(result.stdout), redact(result.stderr)
    except (OSError, subprocess.TimeoutExpired) as error:
        return 127, "", redact(str(error))


def count_turn() -> bool:
    global TURN_CALLS, STOP
    if TURN_CALLS >= MAX_TURN_CALLS:
        STOP = True
        return False
    TURN_CALLS += 1
    return True


def model_call_events(driver: CodexDriver) -> int:
    return driver.model_calls()


def row(run_id: str, trial_id: str, condition: str, **values) -> dict:
    base = {"run_id": run_id, "trial_id": trial_id, "condition": condition,
            "ts_utc": utc_now(), "provider": "codex", "model": CODEX_MODEL,
            "request_result": "observed", "process_id": "none", "thread_id": "none",
            "user_input_request": False, "approval_request": False, "marker_effect": False,
            "private_log": ""}
    base.update(values)
    return base


def config_questions(enabled: bool) -> str:
    return f'''sandbox_mode = "read-only"
approvals_reviewer = "user"

[features]
default_mode_request_user_input = {str(enabled).lower()}
'''


def config_execpolicy() -> str:
    return '''sandbox_mode = "read-only"
approvals_reviewer = "user"

[features]
'''


def rules(decision: str) -> str:
    return f'prefix_rule(pattern = ["touch"], decision = "{decision}")\n'


def contains_method(messages: list[dict], method: str) -> bool:
    return any(message.get("method") == method for message in messages)


def question_trial(run_id: str, trial: int) -> list[dict]:
    trial_id = f"questions-{trial}"
    home = setup_codex_home(trial_id, config_questions(True), "")
    driver = CodexDriver(home)
    events = []
    try:
        init = driver.initialize()
        started = driver.start_thread(approval_policy="on-request", sandbox="read-only")
        thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
        states = [("on-before", True), ("off", False), ("on-after", True)]
        rows = []
        for phase, enabled in states:
            if phase != "on-before":
                changed = driver.request("experimentalFeature/enablement/set", {
                    "enablement": {"default_mode_request_user_input": enabled},
                })
            else:
                changed = None
            if not thread_id:
                rows.append(row(
                    run_id,
                    f"{trial_id}-start",
                    "codex.questions.start",
                    request_result=response_status(started),
                    process_id=str(driver.pid),
                    thread_id="none",
                ))
                break
            if not count_turn():
                break
            started_turn, turn_events = driver.turn(thread_id, question_prompt())
            events.extend(turn_events)
            requests = contains_method(turn_events, "item/tool/requestUserInput")
            rows.append(row(
                run_id,
                f"{trial_id}-{phase}",
                f"codex.questions.{phase}",
                request_result=response_status(started_turn),
                process_id=str(driver.pid),
                thread_id=thread_id or "none",
                user_input_request=requests,
                feature_enabled=enabled,
                enablement_result=response_status(changed) if changed else "initial_config",
            ))
        stderr = driver.close()
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr, "init": init, "thread": started}])
        for item in rows:
            item["private_log"] = log
            item["model_calls"] = model_call_events(driver)
        return rows
    finally:
        if driver.proc.poll() is None:
            driver.close()


def question_reverse_trial(run_id: str, trial: int) -> list[dict]:
    trial_id = f"questions-reverse-{trial}"
    home = setup_codex_home(trial_id, config_questions(False), "")
    driver = CodexDriver(home)
    try:
        init = driver.initialize()
        started = driver.start_thread(approval_policy="on-request", sandbox="read-only")
        thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
        rows = []
        for phase, enabled in (("off-before", False), ("on-after", True)):
            changed = None
            if phase == "on-after":
                changed = driver.request("experimentalFeature/enablement/set", {
                    "enablement": {"default_mode_request_user_input": True},
                })
            if not thread_id:
                rows.append(row(run_id, f"{trial_id}-start", "codex.questions.reverse.start",
                                request_result=response_status(started), process_id=str(driver.pid), thread_id="none"))
                break
            if not count_turn():
                break
            started_turn, turn_events = driver.turn(thread_id, question_prompt())
            rows.append(row(
                run_id,
                f"{trial_id}-{phase}",
                f"codex.questions.reverse.{phase}",
                request_result=response_status(started_turn),
                process_id=str(driver.pid),
                thread_id=thread_id,
                user_input_request=contains_method(turn_events, "item/tool/requestUserInput"),
                feature_enabled=enabled,
                enablement_result=response_status(changed) if changed else "initial_config",
            ))
        stderr = driver.close()
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr, "init": init, "thread": started}])
        for item in rows:
            item["private_log"] = log
            item["model_calls"] = model_call_events(driver)
        return rows
    finally:
        if driver.proc.poll() is None:
            driver.close()


def schema_inventory(run_id: str) -> tuple[dict, list[str]]:
    schema_dir = WORKTREE / ".runtime" / f"schema-{run_id}"
    schema_dir.mkdir(parents=True, exist_ok=True)
    code, stdout, stderr = command(["codex", "app-server", "generate-json-schema", "--out", str(schema_dir)])
    files = sorted(str(path.relative_to(schema_dir)) for path in schema_dir.rglob("*") if path.is_file())
    text = "\n".join(files) + f"\nexit={code}\nstdout={stdout}\nstderr={stderr}\n"
    log = private_log(f"{run_id}-schema.txt", [{"text": text}])
    methods = []
    schema_names = {path.name for path in schema_dir.rglob("*") if path.is_file()}
    if "ConfigBatchWriteParams.json" in schema_names:
        methods.append("config/batchWrite")
    if "ConfigReadParams.json" in schema_names:
        methods.append("config/read")
    if any("rule" in name.lower() or "execpolicy" in name.lower() for name in schema_names):
        methods.append("rule-or-execpolicy-named-schema")
    return {"request_result": "success" if code == 0 else "error", "schema_files": files,
            "schema_sha256": hashlib.sha256(text.encode()).hexdigest(), "private_log": log}, methods


def exec_turn(driver: CodexDriver, thread_id: str, trial_id: str, phase: str, rule_state: str) -> dict:
    marker = marker_path(trial_id, phase)
    if not count_turn():
        return row("", f"{trial_id}-{phase}", "codex.execpolicy.limit", request_result="model_call_cap")
    started, turn_events = driver.turn(thread_id, touch_prompt(marker))
    approvals = [message for message in turn_events if "method" in message and "id" in message and "Approval" in message.get("method", "")]
    approvals.extend(message for message in turn_events if message.get("method") in {"item/commandExecution/requestApproval", "execCommandApproval"})
    return row(
        "", f"{trial_id}-{phase}", f"codex.execpolicy.{rule_state}.{phase}",
        request_result=response_status(started), process_id=str(driver.pid), thread_id=thread_id,
        approval_request=bool(approvals), marker_effect=marker.exists(), rule_state=rule_state,
        reload_path=phase.split("-", 1)[0], approval_methods=sorted({message.get("method") for message in approvals}),
        turn_completed=contains_method(turn_events, "turn/completed"),
    )


def set_rules(home: Path, decision: str) -> None:
    (home / "rules" / "default.rules").write_text(rules(decision), encoding="utf-8")


def execpolicy_trial(run_id: str, trial: int, schema_methods: list[str]) -> list[dict]:
    trial_id = f"execpolicy-{trial}"
    home = setup_codex_home(trial_id, config_execpolicy(), rules("forbidden"))
    driver = CodexDriver(home)
    rows = []
    all_events = []
    try:
        driver.initialize()
        started = driver.start_thread(approval_policy="untrusted", sandbox="read-only")
        thread_id = (started or {}).get("result", {}).get("thread", {}).get("id")
        if not thread_id:
            return [row(run_id, trial_id, "codex.execpolicy.start", request_result=response_status(started))]
        first = exec_turn(driver, thread_id, trial_id, "baseline-forbidden", "forbidden")
        rows.append(first)
        for phase in ("none-allow", "config_batch-allow"):
            set_rules(home, "allow")
            if phase.startswith("config_batch"):
                changed = driver.request("config/batchWrite", {
                    "filePath": str(home / "config.toml"), "edits": [], "reloadUserConfig": True,
                })
                phase_result = response_status(changed)
            else:
                phase_result = "no_request"
            item = exec_turn(driver, thread_id, trial_id, phase, "allow")
            item["reload_request_result"] = phase_result
            rows.append(item)
        for phase in ("none-forbidden", "config_batch-forbidden"):
            set_rules(home, "forbidden")
            if phase.startswith("config_batch"):
                changed = driver.request("config/batchWrite", {
                    "filePath": str(home / "config.toml"), "edits": [], "reloadUserConfig": True,
                })
                phase_result = response_status(changed)
            else:
                phase_result = "no_request"
            item = exec_turn(driver, thread_id, trial_id, phase, "forbidden")
            item["reload_request_result"] = phase_result
            rows.append(item)
        driver.close()

        for state in ("allow", "forbidden"):
            fresh_id = f"{trial_id}-fresh-{state}"
            set_rules(home, state)
            fresh = CodexDriver(home)
            try:
                fresh.initialize()
                fresh_started = fresh.start_thread(approval_policy="untrusted", sandbox="read-only")
                fresh_thread = (fresh_started or {}).get("result", {}).get("thread", {}).get("id")
                if fresh_thread:
                    item = exec_turn(fresh, fresh_thread, fresh_id, f"new-process-{state}", state)
                    item["condition"] = f"codex.execpolicy.new_process.{state}"
                    item["process_id"] = str(fresh.pid)
                    rows.append(item)
            finally:
                stderr = fresh.close()
                all_events.extend(fresh.events)
                fresh_log = private_log(f"{run_id}-{fresh_id}.jsonl", fresh.events + [{"stderr": stderr}])
                for item in rows:
                    if item.get("trial_id") == fresh_id:
                        item["private_log"] = fresh_log
    finally:
        if driver.proc.poll() is None:
            stderr = driver.close()
        else:
            stderr = ""
        log = private_log(f"{run_id}-{trial_id}.jsonl", driver.events + [{"stderr": stderr, "schema_methods": schema_methods}])
        for item in rows:
            if not item.get("private_log"):
                item["private_log"] = log
            item["model_calls"] = model_call_events(driver)
    return rows


def main() -> int:
    commit = subprocess.check_output(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE, text=True).strip()
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + f"-{commit}"
    inventory, schema_methods = schema_inventory(run_id)
    rows = [row(run_id, "inventory-schema", "inventory.codex.schema", **inventory, schema_methods=schema_methods)]
    selected = set(sys.argv[1:]) or {"questions", "execpolicy"}
    if "questions" in selected:
        for trial in range(1, 4):
            if STOP:
                break
            rows.extend(question_trial(run_id, trial))
        for trial in range(1, 4):
            if STOP:
                break
            rows.extend(question_reverse_trial(run_id, trial))
    if "execpolicy" in selected:
        for trial in range(1, 4):
            if STOP:
                break
            rows.extend(execpolicy_trial(run_id, trial, schema_methods))
    RAW.mkdir(parents=True, exist_ok=True)
    path = RAW / f"{run_id}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as stream:
        for value in rows:
            value["run_id"] = run_id
            stream.write(json.dumps(redact(value), ensure_ascii=False, separators=(",", ":")) + "\n")
    env = {"run_id": run_id, "date_utc": utc_now(), "commit": commit, "python": sys.version.split()[0],
           "versions": {name: command([name, "--version"])[1] for name in ("codex",)},
           "models": {"codex": CODEX_MODEL}, "model_call_cap": MAX_TURN_CALLS,
           "turn_start_calls": TURN_CALLS, "stopped_at_cap": STOP}
    (ROOT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"raw": str(path), "turn_start_calls": TURN_CALLS, "stopped_at_cap": STOP}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
