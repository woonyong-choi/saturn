"""실험 252의 네 조건을 실행하고 raw JSON Lines를 만든다."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import random
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

try:
    import tiktoken
except ImportError:
    tiktoken = None

from common import (AppServer, CAP, CLAUDE_MODEL, CODEX_MODEL, PRIVATE, RAW, ROOT, TIMEOUT_S,
                    WORKTREE, append_jsonl, byte_lines, event_lines, json_line, redact, save_private,
                    save_screen, setup_codex_home, thread_params, utc_now, write_jsonl)

SEED = 252
BUDGET_TOKENS = 4000
RECENT_RECORDS = 8
RECOVERY_ROOT = WORKTREE / "experiment-252-recovery"
MCP_FIXTURE = ROOT / "scripts" / "mcp_fixture.py"
CALLS = {"claude": 0, "codex": 0}


def load_handoff():
    path = WORKTREE / "docs" / "experiments" / "handoff-packet-quality-v2" / "scripts" / "01-collect.py"
    spec = importlib.util.spec_from_file_location("handoff_v2", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"기존 실험을 읽지 못했다: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.SEED = SEED
    return module


def check_call(provider: str):
    if CALLS[provider] >= CAP:
        raise RuntimeError(f"{provider} 모델 호출 상한 {CAP}회에 닿았다")
    CALLS[provider] += 1


def packet_parts(scenario: dict) -> tuple[str, list[dict]]:
    users = [row for row in scenario["record"] if row["kind"] == "user"]
    last_user = users[-1]
    fixed = "## Goal and last input\n\n" + last_user["text"] + "\n\n## Recent turns\n\n"
    for user in users[-4:-1]:
        fixed += f"User: {user['text']}\nAgent: 이 단계를 마쳤다.\n\n"
    competing = []
    for row in scenario["record"]:
        if row["seq"] >= last_user["seq"] or row["kind"] == "user" and row is last_user:
            continue
        if row["kind"] == "tool":
            body = f"{row['tool']} {json.dumps(row['args'], ensure_ascii=False)}\nResult:\n{row['result']}"
        else:
            body = f"{row['kind'].title()}: {row.get('text', '')}"
        competing.append({"seq": row["seq"], "kind": row["kind"], "text": body, "session": row["session"]})
    return fixed, competing


def packet_token_count(text: str) -> tuple[int, str]:
    if tiktoken is not None:
        try:
            encoding = tiktoken.get_encoding("o200k_base")
        except (ValueError, LookupError):
            encoding = tiktoken.get_encoding("cl100k_base")
        return len(encoding.encode(text)), f"tiktoken:{encoding.name}"
    return len(text) // 4, "chars/4"


def render_packet(scenario: dict, strategy: str, budget_tokens: int | None = None) -> dict:
    fixed, items = packet_parts(scenario)
    budget = (budget_tokens or BUDGET_TOKENS) * 4
    chosen = list(items)
    protected = set()
    if strategy == "tail-preserve":
        protected.update(item["seq"] for item in items[-RECENT_RECORDS:])
        prior_users = [item for item in items if item["kind"] == "user"]
        if prior_users:
            protected.add(prior_users[-1]["seq"])
    while chosen and len(fixed) + sum(len(item["text"]) + 2 for item in chosen) > budget:
        if strategy == "oldest-first":
            chosen.pop(0)
            continue
        removable = next((item for item in chosen if item["seq"] not in protected and item["kind"] == "tool"), None)
        if removable is None:
            removable = next((item for item in chosen if item["seq"] not in protected), None)
        if removable is None:
            removable = chosen[0]
        chosen.remove(removable)
    text = fixed + "## Earlier records\n\n"
    for item in chosen:
        text += f"#{item['seq']} session={item['session']} {item['text']}\n\n"
    tokens, tokenizer = packet_token_count(text)
    return {"packet": text, "tokens": tokens, "tokenizer": tokenizer,
            "included": [item["seq"] for item in chosen]}


def exp1_manipulation_check(scenarios: list[dict]) -> tuple[int, list[dict]]:
    budget = BUDGET_TOKENS
    while budget >= 250:
        checks = []
        for scenario in scenarios:
            oldest = render_packet(scenario, "oldest-first", budget)
            tail = render_packet(scenario, "tail-preserve", budget)
            checks.append({"scenario_id": scenario["scenario_id"],
                           "budget_tokens": budget,
                           "oldest_included": oldest["included"],
                           "tail_included": tail["included"],
                           "included_diff_count": len(set(oldest["included"]) ^ set(tail["included"])),
                           "oldest_packet_tokens": oldest["tokens"],
                           "tail_packet_tokens": tail["tokens"],
                           "token_diff": tail["tokens"] - oldest["tokens"],
                           "tokenizer": oldest["tokenizer"]})
        if all(row["included_diff_count"] and row["token_diff"] for row in checks):
            return budget, checks
        budget -= 250
    raise RuntimeError("실험 1 조작 확인에서 두 조건 차이를 모든 시나리오에 만들지 못했다")


def prompt_for_packet(packet: str, questions: list[dict]) -> str:
    rules = ("아래 맥락만 근거로 질문에 답하라. 파일을 읽거나 명령을 실행하지 마라. "
             "맥락에 근거가 없으면 unknown을 true로 둔다. 숫자는 숫자만, 날짜는 YYYY-MM-DD로 쓴다. "
             '답은 JSON 배열 하나로만 쓴다: [{"id":"q1","answer":"...","unknown":false}]')
    return rules + "\n\n# 맥락\n\n" + packet + "\n\n# 질문\n\n" + "\n".join(
        f"{q['qid']}. {q['text']}" for q in questions)


def plain_provider(provider: str, prompt: str) -> dict:
    check_call(provider)
    if provider == "claude":
        args = ["claude", "-p", "--model", CLAUDE_MODEL, "--safe-mode", "--tools", "",
                "--no-session-persistence", "--output-format", "json"]
    else:
        args = ["codex", "exec", "-m", CODEX_MODEL, "--ephemeral", "--ignore-user-config",
                "--ignore-rules", "--skip-git-repo-check", "--sandbox", "read-only", "--json", "-"]
    started = time.monotonic()
    try:
        done = subprocess.run(args, input=prompt, capture_output=True, text=True, cwd=WORKTREE,
                              timeout=TIMEOUT_S, check=False)
        result = {"exit_code": done.returncode, "stdout": redact(done.stdout), "stderr": redact(done.stderr),
                  "elapsed_s": round(time.monotonic() - started, 3)}
    except subprocess.TimeoutExpired as error:
        result = {"exit_code": None, "stdout": redact(error.stdout or ""), "stderr": "timeout",
                  "elapsed_s": round(time.monotonic() - started, 3)}
    return result


def parse_provider_answer(provider: str, stdout: str) -> tuple[str, int]:
    if provider == "claude":
        try:
            value = json.loads(stdout)
            return str(value.get("result", "")), int(value.get("num_turns", 1)) - 1
        except (json.JSONDecodeError, AttributeError, TypeError):
            return "", 0
    answer, tools = "", 0
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") or {}
        if event.get("type") == "item.completed" and item.get("type") == "agent_message":
            answer = item.get("text", "")
        if event.get("type") == "item.started" and item.get("type") not in ("agent_message", "reasoning"):
            tools += 1
    return answer, tools


def normalize(text: str) -> str:
    return "".join(str(text).lower().split()).replace(",", "")


def grade(question: dict, answer: dict | None) -> int:
    if answer is None:
        return 0
    if question["qtype"] == "abstain":
        return int(bool(answer.get("unknown")))
    if answer.get("unknown"):
        return 0
    said = normalize(answer.get("answer", ""))
    if not all(normalize(gold) in said for gold in question["gold"]):
        return 0
    return int(not any(normalize(stale) in said for stale in question["stale"]))


def answer_map(text: str) -> dict:
    start, end = text.find("["), text.rfind("]")
    if start < 0 or end < start:
        return {}
    try:
        return {str(item.get("id")): item for item in json.loads(text[start:end + 1])}
    except (json.JSONDecodeError, AttributeError, TypeError):
        return {}


def exp1(run_id: str, limit: int):
    handoff = load_handoff()
    scenarios = [handoff.build_scenario(index) for index in range(16)]
    random.Random(SEED).shuffle(scenarios)
    budget_tokens, manipulation = exp1_manipulation_check(scenarios)
    if limit != len(scenarios):
        manipulation = [row for row in manipulation if row["scenario_id"] in {s["scenario_id"] for s in scenarios[:limit]}]
    rows = []
    for scenario in scenarios[:limit]:
        for provider in ("claude", "codex"):
            order = ["oldest-first", "tail-preserve"]
            random.Random(f"{SEED}-{scenario['scenario_id']}-{provider}").shuffle(order)
            for strategy in order:
                packet = render_packet(scenario, strategy, budget_tokens)
                output = plain_provider(provider, prompt_for_packet(packet["packet"], scenario["questions"]))
                answer_text, tools = parse_provider_answer(provider, output["stdout"])
                answers = answer_map(answer_text)
                results = [{"qid": q["qid"], "qtype": q["qtype"], "correct": grade(q, answers.get(q["qid"]))}
                           for q in scenario["questions"]]
                trial = f"exp1-{scenario['scenario_id']}-{provider}-{strategy}"
                log_path = save_private(trial, [], output["stdout"], output["stderr"])
                rows.append({"run_id": run_id, "trial_id": trial, "condition": strategy, "ts_utc": utc_now(),
                             "scenario_id": scenario["scenario_id"], "provider": provider, "model": output.get("model", provider),
                             "strategy": strategy, "packet_tokens": packet["tokens"], "tokenizer": packet["tokenizer"],
                             "budget_tokens": budget_tokens, "included": packet["included"],
                             "manipulation_check": next(item for item in manipulation
                                                         if item["scenario_id"] == scenario["scenario_id"]),
                             "question_results": results, "answer_text": answer_text, "tool_count": tools,
                             "stdout_sha256": hashlib.sha256(output["stdout"].encode()).hexdigest(),
                             "private_log": log_path, "status": "ok" if output["exit_code"] == 0 and tools == 0 else "invalid",
                             "exit_code": output["exit_code"], "elapsed_s": output["elapsed_s"]})
            print(f"exp1 {scenario['scenario_id']} {provider}", flush=True)
    write_jsonl(RAW / f"exp1-{run_id}.jsonl", rows)


def command_allowed(command: str) -> bool:
    text = command.strip()
    if ".." in text or "/tmp" in text or "/Users" in text or "curl" in text or "rm " in text:
        return False
    return (text.startswith(("ls", "test", "stat", "find", "pwd", "touch", "sleep")) or
            text.startswith("python3 -c"))


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


def approval_handler(server: AppServer, message: dict):
    if message.get("id") is None:
        return
    if message.get("method", "").endswith("requestApproval"):
        server.send_result(message["id"], {"decision": "accept"})
    else:
        server.send_error(message["id"], -32601, "unhandled experiment request")


def claude_stream(trial_id: str, prompt: str, tools: str, ask_answers: bool = False,
                  interrupt: str | None = None) -> tuple[list[dict], str, str]:
    check_call("claude")
    args = ["claude", "-p", "--model", CLAUDE_MODEL, "--safe-mode", "--no-session-persistence",
            "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--permission-mode", "manual", "--permission-prompts", "host", "--permission-prompt-tool", "stdio",
            "--tools", tools]
    proc = subprocess.Popen(args, cwd=WORKTREE, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, start_new_session=True)
    events: list[dict] = []
    user = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": prompt}]}}
    proc.stdin.write((json_line(user) + "\n").encode())
    proc.stdin.flush()
    events.append({"direction": "out", "tsNs": time.time_ns(), "line": json_line(user)})
    interrupted = False
    command_seen = False
    try:
        for text in byte_lines(proc, time.monotonic() + TIMEOUT_S, events):
            try:
                message = json.loads(text)
            except json.JSONDecodeError:
                continue
            if message.get("type") == "control_request":
                request = message.get("request", {})
                name = request.get("tool_name")
                input_value = request.get("input", {}) or {}
                command = input_value.get("command", "")
                command_seen |= bool(command)
                if name == "AskUserQuestion" and ask_answers:
                    answers = {q.get("question", str(i)): "experiment-answer" for i, q in enumerate(input_value.get("questions", []))}
                    response = {"behavior": "allow", "updatedInput": {**input_value, "answers": answers}}
                elif name == "Bash" and command_allowed(command):
                    response = {"behavior": "allow", "updatedInput": input_value}
                else:
                    response = {"behavior": "deny", "message": "실험에서 허용하지 않은 명령"}
                reply = {"type": "control_response", "response": {"subtype": "success",
                         "request_id": message.get("request_id"), "response": response}}
                proc.stdin.write((json_line(reply) + "\n").encode())
                proc.stdin.flush()
                events.append({"direction": "out", "tsNs": time.time_ns(), "line": json_line(reply)})
                if interrupt == "normal" and command_seen and not interrupted:
                    time.sleep(1)
                    proc.send_signal(signal.SIGINT)
                    interrupted = True
                elif interrupt == "force" and command_seen and not interrupted:
                    time.sleep(1)
                    proc.send_signal(signal.SIGINT)
                    time.sleep(2)
                    if proc.poll() is None:
                        os.killpg(proc.pid, signal.SIGKILL)
                    interrupted = True
            if message.get("type") == "result":
                break
    finally:
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
                proc.wait(timeout=2)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        stderr = ""
    save_private(trial_id, events, "", stderr)
    return events, stderr, "interrupted" if interrupted else "ended"


def app_thread(app: AppServer, params: dict) -> str:
    response = app.request("thread/start", params)
    if not response or response.get("error"):
        raise RuntimeError(f"thread/start 실패: {redact(response or {})}")
    result = (response or {}).get("result", {})
    thread = result.get("thread") or {}
    thread_id = thread.get("id") or result.get("threadId") or result.get("id") or ""
    if not thread_id:
        raise RuntimeError(f"thread/start 응답에 thread id가 없다: {redact(response)}")
    return thread_id


def codex_turn(app: AppServer, thread_id: str, prompt: str, handler, timeout: float = TIMEOUT_S,
               interrupt: str | None = None):
    check_call("codex")
    turn_request_id = app.next_id
    app.send("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}, turn_request_id)
    turn_id = {"value": None}
    interrupted = {"value": False}

    def wrapped(server: AppServer, message: dict):
        params = message.get("params", {})
        if message.get("method") == "turn/started":
            turn_id["value"] = params.get("turn", {}).get("id") or params.get("turnId")
        handler(server, message)
        item = params.get("item") or {}
        command_started = message.get("method") == "item/started" and item.get("type") == "commandExecution"
        if interrupt and not interrupted["value"] and (command_started or message.get("method") == "item/commandExecution/requestApproval"):
            time.sleep(1)
            server.send("turn/interrupt", {"threadId": thread_id, "turnId": turn_id["value"]}, server.next_id)
            interrupted["value"] = True
            child_pid = item.get("processId")
            if child_pid:
                try:
                    os.kill(int(child_pid), signal.SIGINT)
                except (ProcessLookupError, ValueError):
                    pass
            if interrupt == "force":
                time.sleep(2)
                if child_pid:
                    try:
                        os.kill(int(child_pid), signal.SIGKILL)
                    except (ProcessLookupError, ValueError):
                        pass
                if server.proc.poll() is None:
                    os.killpg(server.proc.pid, signal.SIGKILL)

    messages = app.drain(timeout, wrapped, stop=lambda msg: msg.get("method") in ("turn/completed", "turn/failed", "error") or
                                                      (msg.get("id") == turn_request_id and "error" in msg))
    return messages, interrupted["value"]


def exp2(run_id: str):
    rows = []
    for provider in ("claude", "codex"):
        conditions = ["no-warning", "state-warning"]
        random.Random(f"{SEED}-{provider}-exp2").shuffle(conditions)
        for condition in conditions:
            for repeat in range(1, 6):
                trial_id = f"exp2-{provider}-{condition}-{repeat}"
                work = RECOVERY_ROOT / trial_id
                work.mkdir(parents=True, exist_ok=True)
                target = work / "worktree-touch.txt"
                target.write_text("already-created\n", encoding="utf-8")
                warning = "중단됐고 일부 실행됐을 수 있으니 상태부터 확인하라. " if condition == "state-warning" else ""
                ending = "계속해서 작업을 마무리하라. 결과를 한 문장으로 알려라."
                prompt = (f"이전 턴은 다음 명령을 실행하던 중 끊겼다. 결과는 기록되지 않았다: touch {target.name}\n"
                          f"{warning}{ending}")
                if provider == "claude":
                    events, stderr, status = claude_stream(trial_id, prompt, "Bash")
                    messages = []
                else:
                    home, env = setup_codex_home("exp2")
                    app = AppServer(env)
                    try:
                        app.initialize()
                        thread = app_thread(app, thread_params(work))
                        messages, _ = codex_turn(app, thread, prompt, approval_handler)
                        stderr = app.close()
                        status = "ended"
                        events = app.events
                    except Exception as error:
                        stderr, status, events, messages = str(error), "error", getattr(app, "events", []), []
                        app.close()
                parsed = messages or event_lines(events)
                commands = command_values(parsed)
                state_first = bool(commands and state_check(commands[0]))
                duplicate = any("touch worktree-touch.txt" in command for command in commands)
                log_path = save_private(trial_id + "-summary", events, "", stderr)
                rows.append({"run_id": run_id, "trial_id": trial_id, "condition": condition, "ts_utc": utc_now(),
                             "provider": provider, "model": CLAUDE_MODEL if provider == "claude" else CODEX_MODEL,
                             "state_checked_first": int(state_first), "duplicate_touch": int(duplicate),
                             "commands": commands, "approval_events": [m.get("method") for m in parsed if m.get("method", "").endswith("requestApproval")],
                             "private_log": log_path, "status": status})
                print(trial_id, flush=True)
    write_jsonl(RAW / f"exp2-{run_id}.jsonl", rows)


def mcp_handler(action: str, request_rows: list[dict]):
    def handler(server: AppServer, message: dict):
        method = message.get("method", "")
        if method == "mcpServer/elicitation/request":
            params = message.get("params", {})
            response = {"action": action}
            if action == "accept":
                response["content"] = {"title": "Saturn", "count": 3, "enabled": True, "choice": "a", "tags": ["x", "y"]}
            request_rows.append({"method": method, "params": params, "response": response})
            server.send_result(message["id"], response)
        elif method == "item/tool/requestUserInput":
            request_rows.append({"method": method, "params": message.get("params")})
            questions = message.get("params", {}).get("questions", [])
            server.send_result(message["id"], {"answers": {q.get("id", str(i)): "experiment-answer" for i, q in enumerate(questions)}})
        elif method.endswith("requestApproval"):
            server.send_result(message["id"], {"decision": "accept"})
        elif message.get("id") is not None:
            server.send_error(message["id"], -32601, "unhandled experiment request")
    return handler


def exp3(run_id: str):
    rows = []
    PRIVATE.mkdir(parents=True, exist_ok=True)
    actions = ["accept", "decline", "cancel"]
    for mode in ("form", "url"):
        for index, action in enumerate(actions, 1):
            trial_id = f"exp3-codex-{mode}-{action}-{index}"
            log = PRIVATE / f"{trial_id}-mcp.jsonl"
            config = ("approval_policy = \"on-request\"\n"
                      "sandbox_mode = \"workspace-write\"\n"
                      "mcp_optional_startup_grace_ms = 12000\n\n"
                      "[mcp_servers.experiment_252]\n"
                      f"command = \"python3\"\nargs = [\"{MCP_FIXTURE}\", \"--mode\", \"{mode}\", \"--log\", \"{log}\"]\n"
                      "startup_timeout_sec = 30\ntool_timeout_sec = 30\nrequired = false\n"
                      "default_tools_approval_mode = \"prompt\"\n\n"
                      "[mcp_servers.experiment_252.tools.request_experiment_input]\n"
                      "approval_mode = \"prompt\"\n")
            _, env = setup_codex_home(f"exp3-{mode}-{action}", config)
            app = AppServer(env, enable=["default_mode_request_user_input"])
            requests = []
            try:
                app.initialize()
                thread = app_thread(app, thread_params(WORKTREE))
                prompt = ("Use the MCP tool request_experiment_input exactly once. Then report the response you received. "
                          f"Use mode={mode}.")
                messages, _ = codex_turn(app, thread, prompt, mcp_handler(action, requests), timeout=120)
                stderr = app.close()
                status = "ok"
            except Exception as error:
                messages, stderr, status = [], str(error), "error"
                app.close()
            rows.append({"run_id": run_id, "trial_id": trial_id, "condition": f"{mode}-{action}", "ts_utc": utc_now(),
                         "provider": "codex", "model": CODEX_MODEL, "request_method": [r["method"] for r in requests],
                         "request_params": [r.get("params") for r in requests], "response": [r.get("response") for r in requests],
                         "server_observation": redact(messages), "round_trip": int(bool(requests and status == "ok")),
                         "private_log": save_private(trial_id, app.events, "", stderr), "status": status})
    feature_trial = "exp3-codex-agent-request"
    _, env = setup_codex_home("exp3-agent")
    app = AppServer(env, enable=["default_mode_request_user_input"])
    requests = []
    try:
        app.initialize()
        thread = app_thread(app, thread_params(WORKTREE))
        messages, _ = codex_turn(app, thread, "Ask the user one question with the request user input tool, then report the answer.", mcp_handler("accept", requests), timeout=120)
        stderr, status = app.close(), "ok"
    except Exception as error:
        messages, stderr, status = [], str(error), "error"
        app.close()
    rows.append({"run_id": run_id, "trial_id": feature_trial, "condition": "agent-request", "ts_utc": utc_now(),
                 "provider": "codex", "model": CODEX_MODEL, "request_method": [r["method"] for r in requests],
                 "request_params": [r.get("params") for r in requests], "response": [], "server_observation": redact(messages),
                 "round_trip": int(any(r["method"] == "item/tool/requestUserInput" for r in requests)),
                 "private_log": save_private(feature_trial, app.events, "", stderr), "status": status,
                 "feature_flag": "default_mode_request_user_input"})
    for index in range(1, 4):
        trial_id = f"exp3-claude-ask-user-{index}"
        events, stderr, status = claude_stream(trial_id,
            "Use AskUserQuestion exactly once to ask for a project name and then repeat the answer.",
            "AskUserQuestion", ask_answers=True)
        messages = event_lines(events)
        requests = [m for m in messages if m.get("type") == "control_request"]
        reflected = any("experiment-answer" in json.dumps(m, ensure_ascii=False) for m in messages)
        rows.append({"run_id": run_id, "trial_id": trial_id, "condition": "claude-AskUserQuestion", "ts_utc": utc_now(),
                     "provider": "claude", "model": CLAUDE_MODEL, "request_method": ["control_request" for _ in requests],
                     "request_params": requests, "response": [m for m in messages if m.get("type") == "control_response"],
                     "server_observation": messages, "round_trip": int(reflected),
                     "private_log": save_private(trial_id, events, "", stderr), "status": status})
    write_jsonl(RAW / f"exp3-{run_id}.jsonl", rows)


def tmux_capture(session: str) -> str:
    result = subprocess.run(["tmux", "capture-pane", "-p", "-t", session, "-S", "-240"],
                            capture_output=True, text=True, check=False)
    return redact(result.stdout)


def tmux_alive(session: str) -> bool:
    return subprocess.run(["tmux", "has-session", "-t", session],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                          check=False).returncode == 0


def tmux_send(session: str, *keys: str) -> None:
    subprocess.run(["tmux", "send-keys", "-t", session, *keys], check=True)


def tmux_send_text(session: str, text: str, provider: str) -> None:
    subprocess.run(["tmux", "send-keys", "-t", session, "-l", text], check=True)
    tmux_send(session, "C-m" if provider == "claude" else "Enter")


def tmux_wait_ready(session: str, provider: str) -> str:
    skipped_update = False
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and tmux_alive(session):
        screen = tmux_capture(session)
        if provider == "codex" and "update available" in screen.lower() and not skipped_update:
            tmux_send(session, "2")
            tmux_send(session, "Enter")
            skipped_update = True
            time.sleep(1)
            continue
        if provider == "claude" and "Claude Code" in screen and "❯" in screen:
            return screen
        if provider == "codex" and ("Pull up a prompt" in screen or "Ask Codex" in screen):
            return screen
        time.sleep(0.5)
    return tmux_capture(session)


def tmux_command_started(screen: str, command: str) -> bool:
    if command not in screen:
        return False
    lines = [line.strip() for line in screen.splitlines() if command in line]
    input_markers = ("❯", "›", ">", "Run this harmless command")
    return any(not any(line.startswith(marker) for marker in input_markers) for line in lines)


def tmux_approve_once(session: str, command: str, approved: bool) -> bool:
    if approved:
        return True
    screen = tmux_capture(session).lower()
    markers = ("allow", "approve", "run command", "do you want", "permission")
    if command.lower() in screen and any(marker in screen for marker in markers):
        tmux_send(session, "y")
        tmux_send(session, "Enter")
        return True
    return False


def tmux_run_trial(run_id: str, provider: str, stop_kind: str, index: int) -> dict:
    check_call(provider)
    session = f"experiment-252-{provider}-{index}"
    command = "sleep 600" if stop_kind == "interrupt" else "trap '' INT; sleep 600"
    prompt = (f"Run this harmless command exactly once and do not stop it yourself: {command}. "
              "Then report when it ends.")
    model = CLAUDE_MODEL if provider == "claude" else CODEX_MODEL
    cli_args = (["claude", "--model", model, "--permission-mode", "manual"]
                if provider == "claude" else ["codex", "--model", model])
    if tmux_alive(session):
        raise RuntimeError(f"이미 존재하는 실험 세션을 건드리지 않는다: {session}")
    before = ""
    after_interrupt = ""
    after_force = ""
    approved = False
    started = False
    status = "ok"
    try:
        subprocess.run(["tmux", "new-session", "-d", "-s", session, "-c", str(ROOT),
                        *cli_args], check=True)
        before = tmux_wait_ready(session, provider)
        tmux_send_text(session, prompt, provider)
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline and tmux_alive(session):
            approved = tmux_approve_once(session, command, approved)
            before = tmux_capture(session)
            if "could not be refreshed" in before.lower() or "sign in again" in before.lower():
                status = "auth-error"
                break
            if tmux_command_started(before, command):
                started = True
                time.sleep(2)
                break
            time.sleep(0.5)
        if not started:
            status = "command-not-started"
        if not tmux_alive(session):
            status = "ended-before-stop"
        stop_key = "Escape" if provider == "claude" else "C-c"
        if started and tmux_alive(session):
            tmux_send(session, stop_key)
            time.sleep(3)
            after_interrupt = tmux_capture(session)
        child_alive_after_grace = int(tmux_alive(session))
        if stop_kind == "force-kill" and started and tmux_alive(session):
            tmux_send(session, "C-c")
            time.sleep(2)
            after_force = tmux_capture(session)
    except (OSError, subprocess.CalledProcessError) as error:
        status = f"error: {error}"
    finally:
        final = tmux_capture(session) if tmux_alive(session) else ""
        if tmux_alive(session):
            subprocess.run(["tmux", "kill-session", "-t", session], check=False)
    screen = ("===== before stop =====\n" + before +
              "\n===== after interrupt =====\n" + after_interrupt)
    if stop_kind == "force-kill":
        screen += "\n===== after force key =====\n" + after_force
    screen += "\n===== final =====\n" + final
    trial_id = f"exp4-{run_id}-{provider}-{stop_kind}-{index}"
    raw_path = ROOT / "data" / "raw" / f"{trial_id}.txt"
    raw_path.write_text(redact(screen), encoding="utf-8")
    private_path = save_screen(trial_id, screen)
    return {"trial_id": trial_id, "condition": stop_kind, "ts_utc": utc_now(),
            "provider": provider, "model": model, "stop_kind": stop_kind,
            "display_text": screen, "screen_path": private_path,
            "raw_screen_path": str(raw_path.relative_to(WORKTREE)),
            "tmux_session": session, "child_alive_after_grace": child_alive_after_grace,
            "started": int(started), "status": status}


def exp4(run_id: str):
    rows = []
    for provider in ("claude", "codex"):
        for stop_kind in ("interrupt", "force-kill"):
            for index in range(1, 4):
                row = tmux_run_trial(run_id, provider, stop_kind, index)
                row["run_id"] = run_id
                rows.append(row)
                print(row["trial_id"], flush=True)
    write_jsonl(RAW / f"exp4-{run_id}.jsonl", rows)


def env_snapshot(run_id: str) -> dict:
    def version(command):
        done = subprocess.run(command, capture_output=True, text=True, check=False)
        return redact((done.stdout or done.stderr).strip())
    commit = subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE,
                            capture_output=True, text=True, check=False).stdout.strip()
    return {"run_id": run_id, "run_date": utc_now(), "commit": commit, "seed": SEED,
            "tools": {"python": sys.version.split()[0], "codex": version(["codex", "--version"]),
                      "claude": version(["claude", "--version"])},
            "models": {"claude": CLAUDE_MODEL, "codex": CODEX_MODEL}, "calls": CALLS.copy()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", default="all", help="쉼표로 구분한 실험 번호 또는 all")
    parser.add_argument("--limit", type=int, default=16)
    args = parser.parse_args()
    selected = {item.strip() for item in args.only.split(",")}
    if selected == {"all"}:
        selected = {"exp1", "exp2", "exp3", "exp4"}
    if not selected <= {"exp1", "exp2", "exp3", "exp4"}:
        raise SystemExit("--only는 exp1,exp2,exp3,exp4 조합이다")
    if args.limit < 1 or args.limit > 16:
        raise SystemExit("--limit은 1~16이다")
    RAW.mkdir(parents=True, exist_ok=True)
    run_id = f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-{subprocess.run(['git', 'rev-parse', '--short=7', 'HEAD'], cwd=WORKTREE, capture_output=True, text=True).stdout.strip()}"
    try:
        if "exp1" in selected:
            exp1(run_id, args.limit)
        if "exp2" in selected:
            exp2(run_id)
        if "exp3" in selected:
            exp3(run_id)
        if "exp4" in selected:
            exp4(run_id)
        (ROOT / "env.json").write_text(json.dumps(env_snapshot(run_id), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(run_id)
    finally:
        shutil.rmtree(RECOVERY_ROOT, ignore_errors=True)
        shutil.rmtree(WORKTREE / ".runtime", ignore_errors=True)


if __name__ == "__main__":
    main()
