#!/usr/bin/env python3
from __future__ import annotations

import json
import platform
import subprocess
import sys
import time
import uuid
from pathlib import Path

from common import (
    CLAUDE_CAP,
    CLAUDE_MODEL,
    CODEX_CAP,
    CODEX_MODEL,
    MARKER_WAIT_SECONDS,
    POST_RESUME_WAIT_SECONDS,
    ROOT,
    WORKTREE,
    ClaudeDriver,
    claude_work_dir,
    CodexDriver,
    event_summary,
    marker_counts,
    marker_paths,
    marker_prompt,
    model_call_count,
    private_log,
    response_status,
    setup_claude_config,
    setup_codex_home,
    utc_now,
)

RAW = ROOT / "data" / "raw"
RUN_ID = f"{utc_now().replace('-', '').replace(':', '')}-{subprocess.check_output(['git', 'rev-parse', '--short=7', 'HEAD'], cwd=WORKTREE, text=True).strip()}"
CALLS = {"codex": 0, "claude": 0}
RECOLLECT_CALL_LIMIT = 40
COUNTER = WORKTREE / ".runtime" / "claude-call-count"


def take_call() -> int:
    """Claude 실행(`claude` 프로세스 시작) 직전에 센다. 상한에 닿으면 멈춘다."""
    COUNTER.parent.mkdir(parents=True, exist_ok=True)
    used = int(COUNTER.read_text()) if COUNTER.exists() else 0
    if used >= RECOLLECT_CALL_LIMIT:
        raise SystemExit(f"claude call limit {RECOLLECT_CALL_LIMIT} reached")
    COUNTER.write_text(str(used + 1))
    return used + 1


def parent_tool_events(events: list[dict]) -> int:
    return sum(
        1
        for event in events
        if event.get("direction") == "in"
        and event.get("message", {}).get("parent_tool_use_id")
    )


def command_version(name: str) -> dict:
    result = subprocess.run(
        [name, "--version"], cwd=WORKTREE, capture_output=True, text=True, check=False
    )
    return {
        "exit_code": result.returncode,
        "stdout": result.stdout.strip(),
        "stderr": result.stderr.strip(),
    }


def write_env() -> None:
    ROOT.joinpath("env.json").write_text(
        json.dumps(
            {
                "run_id": RUN_ID,
                "ts_utc": utc_now(),
                "os": platform.platform(),
                "machine": platform.machine(),
                "python": platform.python_version(),
                "worktree": str(WORKTREE),
                "models": {"codex": CODEX_MODEL, "claude": CLAUDE_MODEL},
                "versions": {
                    "codex": command_version("codex"),
                    "claude": command_version("claude"),
                },
                "model_call_caps": {"codex": CODEX_CAP, "claude": CLAUDE_CAP},
                "model_calls": CALLS,
                "auth_policy": "전용 home 안에 원본 auth/credentials 심볼릭 링크만 사용; 내용은 기록하지 않음",
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def row(trial_id: str, condition: str, provider: str, model: str, **values) -> dict:
    result = {
        "run_id": RUN_ID,
        "trial_id": trial_id,
        "condition": condition,
        "ts_utc": utc_now(),
        "provider": provider,
        "model": model,
        "request_result": "observed",
        "marker_start_count": 0,
        "marker_complete_count": 0,
        "marker_touch_count": 0,
        "resume_child_execution": None,
        "observation_status": "cannot_distinguish",
        "private_log": "",
    }
    result.update(values)
    return result


def codex_child_id(message: dict, root_id: str) -> str | None:
    if message.get("method") != "thread/started":
        return None
    thread = message.get("params", {}).get("thread", {})
    if thread.get("parentThreadId") == root_id:
        return thread.get("id")
    return None


def codex_turn_id(thread: dict) -> str | None:
    turns = thread.get("turns") or []
    if not turns:
        return None
    return turns[-1].get("id")


def wait_codex_child(
    driver: CodexDriver, root_id: str, events_path: Path
) -> tuple[str | None, int]:
    child_id = None
    deadline = time.monotonic() + MARKER_WAIT_SECONDS
    while time.monotonic() < deadline:
        for message in driver.pump(0.25):
            found = codex_child_id(message, root_id)
            if found:
                child_id = found
            if (
                marker_counts(events_path, events_path.with_suffix(".done"))["start"]
                > 0
            ):
                if child_id is None:
                    thread_ids = {
                        event.get("message", {}).get("params", {}).get("threadId")
                        for event in driver.events
                        if event.get("direction") == "in"
                    }
                    child_id = next(
                        (value for value in thread_ids if value and value != root_id),
                        None,
                    )
                return child_id, model_call_count(driver.events, "codex")
        if driver.proc.poll() is not None:
            break
    return child_id, model_call_count(driver.events, "codex")


def codex_trial(condition: str, trial_number: int, cleaned: bool) -> dict:
    trial_id = f"codex-{condition.replace('.', '-')}-{trial_number}"
    events_path, done_path = marker_paths(trial_id)
    prompt = marker_prompt(
        events_path,
        done_path,
        trial_id,
        "Use the agent tool to start exactly one child agent. Tell the child to",
    )
    home = setup_codex_home(trial_id)
    first = CodexDriver(home)
    root_id = None
    child_id = None
    start_response = None
    first_stderr = ""
    try:
        first.initialize()
        start_response = first.start_thread()
        root_id = (start_response or {}).get("result", {}).get("thread", {}).get("id")
        if root_id:
            first.send(
                {
                    "jsonrpc": "2.0",
                    "id": first.next_id,
                    "method": "turn/start",
                    "params": {
                        "threadId": root_id,
                        "input": [{"type": "text", "text": prompt}],
                    },
                }
            )
            first.next_id += 1
            child_id, _ = wait_codex_child(first, root_id, events_path)
        first.kill_self()
    finally:
        first_stderr = first.close()

    first_counts = marker_counts(events_path, done_path)
    cleanup = []
    resumed = None
    resume_response = None
    second = CodexDriver(home)
    second_stderr = ""
    try:
        second.initialize()
        if cleaned and root_id:
            listed = second.request(
                "thread/list",
                {
                    "cwd": str(WORKTREE),
                    "archived": False,
                    "limit": 100,
                    "sourceKinds": [
                        "cli",
                        "vscode",
                        "exec",
                        "appServer",
                        "subAgent",
                        "subAgentReview",
                        "subAgentCompact",
                        "subAgentThreadSpawn",
                        "subAgentOther",
                        "unknown",
                    ],
                },
            )
            children = [
                item
                for item in (listed or {}).get("result", {}).get("data", [])
                if item.get("id") == child_id or item.get("parentThreadId") == root_id
            ]
            if children:
                child_id = children[0].get("id")
            child = None
            if child_id:
                read = second.request(
                    "thread/read", {"threadId": child_id, "includeTurns": True}
                )
                child = (read or {}).get("result", {}).get("thread")
                turn_id = codex_turn_id(child or {})
                if turn_id:
                    response = second.request(
                        "turn/interrupt",
                        {"threadId": child_id, "turnId": turn_id},
                        timeout=15,
                    )
                    cleanup.append(
                        {
                            "method": "turn/interrupt",
                            "status": response_status(response),
                        }
                    )
                response = second.request(
                    "thread/archive", {"threadId": child_id}, timeout=15
                )
                cleanup.append(
                    {"method": "thread/archive", "status": response_status(response)}
                )
                response = second.request(
                    "thread/unsubscribe", {"threadId": child_id}, timeout=15
                )
                cleanup.append(
                    {
                        "method": "thread/unsubscribe",
                        "status": response_status(response),
                    }
                )
        if root_id:
            resume_response = second.request(
                "thread/resume", {"threadId": root_id}, timeout=45
            )
            deadline = time.monotonic() + POST_RESUME_WAIT_SECONDS
            while time.monotonic() < deadline:
                second.pump(0.25)
            final_counts = marker_counts(events_path, done_path)
            resumed = final_counts["start"] > first_counts["start"]
    finally:
        second_stderr = second.close()

    final_counts = marker_counts(events_path, done_path)
    CALLS["codex"] += model_call_count(first.events, "codex") + model_call_count(
        second.events, "codex"
    )
    log = private_log(
        f"{RUN_ID}-{trial_id}.jsonl",
        [
            {"phase": "initial", "events": first.events, "stderr": first_stderr},
            {"phase": "resume", "events": second.events, "stderr": second_stderr},
            {
                "root_thread_id": root_id,
                "child_thread_id": child_id,
                "start_response": start_response,
                "resume_response": resume_response,
                "cleanup": cleanup,
                "initial_counts": first_counts,
                "final_counts": final_counts,
                "event_summary_initial": event_summary(first.events),
                "event_summary_resume": event_summary(second.events),
            },
        ],
    )
    return row(
        trial_id,
        f"codex.{condition}",
        "codex",
        CODEX_MODEL,
        request_result=response_status(start_response),
        marker_start_count=final_counts["start"],
        marker_complete_count=final_counts["complete"],
        marker_touch_count=final_counts["touch"],
        resume_child_execution=resumed,
        observation_status="confirmed" if resumed is not None else "cannot_distinguish",
        root_thread_id=root_id,
        child_thread_id=child_id,
        cleanup_requests=cleanup,
        cleanup_used=cleaned,
        resume_request_status=response_status(resume_response),
        model_calls=model_call_count(first.events, "codex")
        + model_call_count(second.events, "codex"),
        private_log=log,
    )


def subagent_bash_events(events: list[dict]) -> int:
    """하위 에이전트(parent_tool_use_id가 있는 메시지)가 낸 Bash 호출 수."""
    count = 0
    for event in events:
        message = event.get("message", {})
        if event.get("direction") != "in" or not message.get("parent_tool_use_id"):
            continue
        content = (message.get("message") or {}).get("content")
        if isinstance(content, list):
            count += sum(
                1
                for item in content
                if item.get("type") == "tool_use" and item.get("name") == "Bash"
            )
    return count


def claude_trial(
    condition: str,
    trial_number: int,
    resume_env: bool,
    task: bool = False,
    default_login: bool = False,
) -> dict:
    trial_id = f"claude-{condition.replace('.', '-')}-{trial_number}"
    events_path, done_path = marker_paths(trial_id)
    session_id = str(uuid.uuid4())
    work = claude_work_dir(trial_id) if default_login else None
    config = None if default_login else setup_claude_config(trial_id)
    if default_login:
        take_call()
    first = ClaudeDriver(
        session_id, config, resume=False, resume_env=resume_env, task=task, work=work
    )
    first_init = False
    first_stderr = ""
    try:
        task_text = (
            (
                "Use the Agent tool (also called Task) to start exactly one"
                " subagent, and tell it to"
                if default_login
                else "Use the Task tool to start exactly one subagent, and tell it to"
            )
            if task
            else "Run Bash and"
        )
        first.send_user(
            marker_prompt(
                events_path, done_path, trial_id, task_text, absolute=default_login
            )
        )
        deadline = time.monotonic() + MARKER_WAIT_SECONDS
        while (
            time.monotonic() < deadline
            and marker_counts(events_path, done_path)["start"] == 0
        ):
            for message in first.pump(0.25):
                if message.get("type") == "system" and message.get("subtype") == "init":
                    first_init = True
                if message.get("type") == "control_request":
                    first.allow_control_request(message)
            if first.proc.poll() is not None:
                break
        first.kill_self()
    finally:
        first_stderr = first.close()

    initial_counts = marker_counts(events_path, done_path)
    resumed = None
    if default_login:
        take_call()
    second = ClaudeDriver(
        session_id, config, resume=True, resume_env=resume_env, task=task, work=work
    )
    second_init = False
    resume_alive = False
    second_stderr = ""
    try:
        deadline = time.monotonic() + POST_RESUME_WAIT_SECONDS
        while time.monotonic() < deadline:
            for message in second.pump(0.25):
                if message.get("type") == "system" and message.get("subtype") == "init":
                    second_init = True
                if message.get("type") == "control_request":
                    second.allow_control_request(message)
        final_counts = marker_counts(events_path, done_path)
        resumed = final_counts["start"] > initial_counts["start"]
        resume_alive = second.proc.poll() is None
    finally:
        second_stderr = second.close()

    final_counts = marker_counts(events_path, done_path)
    child_bash = subagent_bash_events(first.events)
    # 기본 로그인 재수집: 재개 process가 init 없이 관찰 구간 내내 살아 있고 stderr가 비어 있으면
    # 세션을 열고 아무것도 하지 않은 것으로 본다. Task 탐색은 하위 에이전트 Bash가 있어야 유효하다.
    observed_ok = first_init and (second_init or (resume_alive and not second_stderr))
    if default_login and task and child_bash == 0:
        resumed = None
    CALLS["claude"] += model_call_count(first.events, "claude") + model_call_count(
        second.events, "claude"
    )
    log = private_log(
        f"{RUN_ID}-{trial_id}.jsonl",
        [
            {"phase": "initial", "events": first.events, "stderr": first_stderr},
            {"phase": "resume", "events": second.events, "stderr": second_stderr},
            {
                "session_id": session_id,
                "initialised": first_init,
                "resume_initialised": second_init,
                "resume_env_present": resume_env,
                "default_login": default_login,
                "resume_alive_at_end": resume_alive,
                "initial_subagent_bash": child_bash,
                "initial_counts": initial_counts,
                "final_counts": final_counts,
                "event_summary_initial": event_summary(first.events),
                "event_summary_resume": event_summary(second.events),
            },
        ],
    )
    return row(
        trial_id,
        f"claude.{condition}",
        "claude",
        CLAUDE_MODEL,
        request_result=(
            "observed"
            if (observed_ok if default_login else first_init and second_init)
            else "error_or_timeout"
        ),
        marker_start_count=final_counts["start"],
        marker_complete_count=final_counts["complete"],
        marker_touch_count=final_counts["touch"],
        resume_child_execution=resumed,
        observation_status="confirmed" if resumed is not None else "cannot_distinguish",
        session_id=session_id,
        resume_env_present=resume_env,
        task_subagent=task,
        default_login=default_login,
        resume_process_alive=resume_alive,
        resume_init_event=second_init,
        initial_subagent_bash=child_bash,
        resume_parent_tool_events=parent_tool_events(second.events),
        resume_reason=next(
            (
                event["message"].get("resume_reason")
                for event in second.events
                if event.get("direction") == "in"
                and event.get("message", {}).get("resume_reason")
            ),
            None,
        ),
        model_calls=model_call_count(first.events, "claude")
        + model_call_count(second.events, "claude"),
        private_log=log,
    )


def recollect_claude() -> None:
    """공식 CLI 기본 로그인으로 Claude 본 조건 6회와 Task 탐색 3회를 다시 수집한다."""
    RAW.mkdir(parents=True, exist_ok=True)
    path = RAW / f"crash-resume-{RUN_ID}.jsonl"
    if path.exists():
        path.unlink()
    plan = [
        ("resume-env-present", True, False),
        ("resume-env-absent", False, False),
        ("task-subagent", False, True),
    ]
    if "--only" in sys.argv:
        keep = sys.argv[sys.argv.index("--only") + 1].split(",")
        plan = [item for item in plan if item[0] in keep]
    for condition, enabled, task in plan:
        for trial in range(1, 4):
            value = claude_trial(
                condition, trial, enabled, task=task, default_login=True
            )
            value["recollect_of"] = "20261003T085117Z-0e500f0"
            with path.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write(
                    json.dumps(
                        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
                    )
                    + "\n"
                )
    env_path = ROOT / "env.json"
    env = json.loads(env_path.read_text(encoding="utf-8"))
    runs = env.get("claude_recollect", [])
    if isinstance(runs, dict):
        runs = [runs]
    runs.append({
        "run_id": RUN_ID,
        "ts_utc": utc_now(),
        "version": command_version("claude"),
        "model": CLAUDE_MODEL,
        "login": "공식 CLI 기본 로그인(CLAUDE_CONFIG_DIR 없음, credentials 링크 없음)",
        "cwd": ".runtime/claude-work/<trial>",
        "claude_launches_cumulative": int(COUNTER.read_text()),
        "launch_limit": RECOLLECT_CALL_LIMIT,
        "model_calls": CALLS["claude"],
        "conditions": [item[0] for item in plan],
    })
    env["claude_recollect"] = runs
    env_path.write_text(
        json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def collect() -> None:
    RAW.mkdir(parents=True, exist_ok=True)
    path = RAW / f"crash-resume-{RUN_ID}.jsonl"
    if path.exists():
        path.unlink()
    rows = []

    def record(value: dict) -> None:
        rows.append(value)
        with path.open("a", encoding="utf-8", newline="\n") as stream:
            stream.write(
                json.dumps(
                    value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
                )
                + "\n"
            )

    for condition, cleaned in (("raw-resume", False), ("cleaned-resume", True)):
        for trial in range(1, 4):
            if CALLS["codex"] >= CODEX_CAP:
                break
            record(codex_trial(condition, trial, cleaned))
            if CALLS["codex"] >= CODEX_CAP:
                break
    for condition, enabled in (
        ("resume-env-present", True),
        ("resume-env-absent", False),
    ):
        for trial in range(1, 4):
            if CALLS["claude"] >= CLAUDE_CAP:
                break
            record(claude_trial(condition, trial, enabled))
            if CALLS["claude"] >= CLAUDE_CAP:
                break
    if CALLS["claude"] < CLAUDE_CAP:
        record(claude_trial("task-subagent", 1, False, task=True))
    write_env()
    env_path = ROOT / "env.json"
    env = json.loads(env_path.read_text(encoding="utf-8"))
    env["model_calls"] = CALLS
    env_path.write_text(
        json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    if "--claude-default-login" in sys.argv:
        recollect_claude()
    else:
        collect()
