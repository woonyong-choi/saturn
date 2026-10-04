#!/usr/bin/env python3
"""실제 Codex app-server로 자식 세션, 끼워 넣기, 승인 응답, 샌드박스, 설정 보고, 훅을 잰다.

사용: 01-collect.py <주제>... (child steer approval sandbox settings hook auth). 주제를 생략하면 전부 돈다.
판정은 process 단계가 raw의 이벤트·승인 기록·파일 효과로 한다. 여기서는 관측만 모은다.
"""
from __future__ import annotations

import copy
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (  # noqa: E402
    CALL_LOG, MODEL, ROOT, RUNTIME, WORKTREE, AppServer, calls_used, jline, make_home, observe,
    record_call, turn_events, utc_now,
)

TRIALS = 3
FAKE_ENTRY = "saturn-test-dummy"
NOISE = {
    "item/agentMessage/delta", "item/reasoning/summaryTextDelta", "item/reasoning/summaryPartAdded",
    "item/reasoning/textDelta", "item/commandExecution/outputDelta", "item/fileChange/outputDelta",
    "item/fileChange/patchUpdated", "account/rateLimits/updated", "thread/tokenUsage/updated",
    "remoteControl/status/changed", "item/plan/delta",
}
SANDBOX_DENIAL = ("read-only file system", "operation not permitted", "permission denied")


def trunc(value, limit=240):
    if isinstance(value, dict):
        return {key: trunc(item, limit) for key, item in value.items()}
    if isinstance(value, list):
        return [trunc(item, limit) for item in value]
    if isinstance(value, str) and len(value) > limit:
        return value[:limit] + f"...[{len(value)}자]"
    return value


def compact(events: list[dict]) -> list[dict]:
    if not events:
        return []
    start = events[0]["ns"]
    out = []
    for event in events:
        message = event["msg"]
        method = message.get("method")
        if method in NOISE or (method and method.endswith("Delta")):
            continue
        out.append({"ms": (event["ns"] - start) // 1_000_000, "dir": event["dir"], "msg": trunc(message)})
    return out


def incoming(server: AppServer) -> list[dict]:
    return [event["msg"] for event in server.events if event["dir"] == "in"]


class Approver:
    """승인 요청마다 사전에 정한 규칙으로 답하고 요청 원문을 기록한다."""

    def __init__(self, server, commands=(), files=False, command_decisions=("accept",), file_decisions=("accept",),
                 mcp_decisions=({"action": "decline"},), max_commands=6, hold_first_file=False):
        self.server = server
        self.commands = tuple(commands)
        self.files = files
        self.command_decisions = list(command_decisions)
        self.file_decisions = list(file_decisions)
        self.mcp_decisions = list(mcp_decisions)
        self.max_commands = max_commands
        self.hold_first_file = hold_first_file
        self.log: list[dict] = []
        self.counts = {"command": 0, "file": 0, "mcp": 0}
        self.held = None
        self.held_at = None
        self.first_accept_at: dict[str, float] = {}

    def _pick(self, decisions, index):
        return decisions[min(index, len(decisions) - 1)]

    def __call__(self, msg):
        method = msg["method"]
        params = msg.get("params", {})
        response = None
        error = None
        if method == "item/commandExecution/requestApproval":
            command = str(params.get("command", ""))
            index = self.counts["command"]
            self.counts["command"] += 1
            if self.commands and any(token in command for token in self.commands) and index < self.max_commands:
                response = {"decision": self._pick(self.command_decisions, index)}
            else:
                response = {"decision": "decline"}
        elif method == "item/fileChange/requestApproval":
            index = self.counts["file"]
            self.counts["file"] += 1
            if self.hold_first_file and self.held is None and index == 0:
                self.held = msg
                self.held_at = time.monotonic()
                self.log.append({"method": method, "id": msg["id"], "threadId": params.get("threadId"), "params": trunc(params), "response": "held"})
                return
            response = {"decision": self._pick(self.file_decisions, index) if self.files else "decline"}
        elif method == "mcpServer/elicitation/request":
            index = self.counts["mcp"]
            self.counts["mcp"] += 1
            response = self._pick(self.mcp_decisions, index)
        else:
            error = {"code": -32601, "message": "driver does not handle this request"}
        thread = params.get("threadId")
        if response is not None and response.get("decision") in ("accept", "acceptForSession") and thread not in self.first_accept_at:
            self.first_accept_at[thread] = time.monotonic()
        self.log.append({"method": method, "id": msg["id"], "threadId": thread, "params": trunc(params), "response": response if response is not None else error})
        self.server.respond(msg, response, error)

    def release(self, decision="accept"):
        if self.held is not None:
            held, self.held = self.held, None
            thread = held["params"].get("threadId")
            self.first_accept_at.setdefault(thread, time.monotonic())
            self.server.respond(held, {"decision": decision})
            self.log[-1]["released_response"] = decision


def descendants(root_pid: int) -> list[tuple[int, str]]:
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,command="], capture_output=True, text=True, errors="replace").stdout
    table = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        table[int(parts[0])] = (int(parts[1]), parts[2])
    found, queue = [], [root_pid]
    while queue:
        parent = queue.pop()
        for pid, (ppid, command) in table.items():
            if ppid == parent and (pid, command) not in found:
                found.append((pid, command))
                queue.append(pid)
    return found


def count_processes(root_pid: int, needle: str) -> int:
    return sum(1 for _, command in descendants(root_pid) if needle in command and not command.startswith("ps "))


def kill_descendants(root_pid: int) -> None:
    for pid, _ in descendants(root_pid):
        try:
            os.kill(pid, 15)
        except ProcessLookupError:
            pass


ACTIVE: list = []


class Trial:
    """전용 CODEX_HOME, 전용 작업 폴더, app-server 하나, thread 하나."""

    def __init__(self, run_id, topic, cond, n, config="", rules="", hooks=None, sandbox="read-only", approval="untrusted",
                 writable=(), files=None, link_auth=True, make_thread=True, git_commit=False):
        self.run_id, self.topic, self.cond, self.n = run_id, topic, cond, n
        self.trial_id = f"{cond}-{n}"
        self.work = RUNTIME / "work" / run_id / self.trial_id
        self.outside = RUNTIME / "outside" / run_id / self.trial_id
        self.work.mkdir(parents=True, exist_ok=True)
        self.outside.mkdir(parents=True, exist_ok=True)
        for name, text in (files or {}).items():
            path = self.work / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        env = {**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null"}
        subprocess.run(["git", "init", "-q"], cwd=self.work, check=True, env=env)
        if git_commit:
            for cmd in (["git", "-c", "user.email=t@t", "-c", "user.name=t", "add", "-A"],
                        ["git", "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "base"]):
                subprocess.run(cmd, cwd=self.work, check=True, env=env)
        self.home = make_home(f"{run_id}/{self.trial_id}", config=config, rules=rules, hooks=hooks, link_auth=link_auth)
        self.server = AppServer(self.home, self.work)
        ACTIVE.append(self.server)
        self.facts: dict = {}
        self.calls: list[int] = []
        self.init = self.server.initialize()
        self.thread_response = None
        self.thread_id = None
        if make_thread:
            params = {"cwd": str(self.work), "model": MODEL, "approvalPolicy": approval, "sandbox": sandbox, "ephemeral": False}
            if writable:
                params["config"] = {"sandbox_workspace_write": {"writable_roots": [str(p) for p in writable]}}
            self.thread_response = self.server.request("thread/start", params)
            self.thread_id = ((self.thread_response or {}).get("result") or {}).get("thread", {}).get("id")

    def call(self, kind="turn/start") -> int:
        ordinal = record_call(self.run_id, self.trial_id, kind)
        self.calls.append(ordinal)
        return ordinal

    def finish(self, status, approver=None, extra=None) -> dict:
        pid = self.server.pid
        kill_descendants(pid)
        stderr = self.server.close()
        row = {
            "run_id": self.run_id, "trial_id": self.trial_id, "topic": self.topic, "condition": self.cond,
            "ts_utc": utc_now(), "provider": "codex", "model": MODEL, "process_id": str(pid),
            "thread_id": self.thread_id, "status": status, "call_ordinals": self.calls,
            "approvals": approver.log if approver else [], "facts": self.facts,
            "events": compact(self.server.events), "stderr_head": trunc(stderr, 600),
        }
        row.update(extra or {})
        return row


def file_exists(path: Path) -> bool:
    return path.exists()


def command_results(server: AppServer) -> list[dict]:
    results = []
    for message in incoming(server):
        if message.get("method") == "item/completed":
            item = message["params"]["item"]
            if item.get("type") == "commandExecution":
                output = str(item.get("aggregatedOutput") or "")
                results.append({
                    "thread": message["params"].get("threadId"), "command": trunc(str(item.get("command")), 200), "status": item.get("status"),
                    "exitCode": item.get("exitCode"), "output": trunc(output, 300),
                    "sandbox_denial": any(word in output.lower() for word in SANDBOX_DENIAL),
                })
    return results


def turn_id_of(server: AppServer, thread_id: str, index=0):
    ids = [m["params"]["turn"]["id"] for m in incoming(server) if m.get("method") == "turn/started" and m["params"].get("threadId") == thread_id]
    return ids[index] if len(ids) > index else None


# ---------------------------------------------------------------- child (#20, #348 subagent)

CHILD_WAIT = ("Use the agent tool to start exactly one child agent. Tell the child to run exactly this shell command and nothing else: "
              "sleep 6; echo child-done. Wait for the child to finish, then reply with the single word finished.")
CHILD_DETACHED = ("Use the agent tool to start exactly one child agent. Tell the child to run exactly this shell command and nothing else: "
                  "sleep 20; echo child-done. Do not wait for the child and do not call the wait tool. Reply with the single word started right after the child is created.")
CHILD_LONG = ("Use the agent tool to start exactly one child agent. Tell the child to run exactly this shell command and nothing else: "
              "sleep 47; echo child-done. Wait for the child to finish, then reply with the single word finished.")


def child_ids(server: AppServer, parent: str) -> list[str]:
    ids = []
    for message in incoming(server):
        thread = (message.get("params") or {}).get("threadId")
        if thread and thread != parent and thread not in ids and message.get("method") not in ("account/updated",):
            ids.append(thread)
    return ids


def run_child(run_id, cond, n, prompt, config="", stop=False):
    trial = Trial(run_id, "child", cond, n, config=config)
    server = trial.server
    approver = Approver(server, commands=("sleep",))
    state = {"stage": 0, "t": None, "child": None}

    def on_message(msg):
        if not stop:
            return False
        parent = trial.thread_id
        if state["stage"] == 0:
            child = child_ids(server, parent)
            first_accept = [t for t in approver.first_accept_at if t != parent]
            if child and first_accept:
                state["child"] = child[0]
                if time.monotonic() - approver.first_accept_at[first_accept[0]] >= 3:
                    state["stage"] = 1
                    state["t"] = time.monotonic()
                    state["alive_before"] = count_processes(server.pid, "sleep 47")
                    server.send_async("turn/interrupt", {"threadId": parent, "turnId": turn_id_of(server, parent)})
        return False

    trial.call()
    _, status, _ = turn_events(server, trial.thread_id, prompt, approver, timeout=170, on_message=on_message)
    trial.facts["parent_turn_status"] = status
    if stop:
        parent = trial.thread_id
        t_int = state["t"] or time.monotonic()
        observe(server, max(0.0, 2 - (time.monotonic() - t_int)), approver)
        trial.facts["sleep_alive_at_2s"] = count_processes(server.pid, "sleep 47")
        observe(server, max(0.0, 12 - (time.monotonic() - t_int)), approver)
        trial.facts["sleep_alive_at_12s"] = count_processes(server.pid, "sleep 47")
        trial.facts["parent_interrupt_sent"] = state["stage"] == 1
        child = state["child"]
        trial.facts["child_thread"] = child
        child_done = [m for m in incoming(server) if m.get("method") == "turn/completed" and m["params"].get("threadId") == child]
        trial.facts["child_completed_before_child_interrupt"] = bool(child_done)
        if child and not child_done:
            child_turn = turn_id_of(server, child)
            trial.facts["child_interrupt_turn"] = child_turn
            if child_turn:
                server.send_async("turn/interrupt", {"threadId": child, "turnId": child_turn})
            observe(server, 10, approver)
            trial.facts["sleep_alive_after_child_interrupt"] = count_processes(server.pid, "sleep 47")
    elif cond == "detached_child":
        observe(server, 40, approver, on_message=lambda m: any(
            x.get("method") == "turn/completed" and x["params"].get("threadId") != trial.thread_id for x in incoming(server)))
    else:
        observe(server, 3, approver)
    trial.facts["child_threads"] = child_ids(server, trial.thread_id)
    trial.facts["multi_agent_mode"] = ((trial.thread_response or {}).get("result") or {}).get("multiAgentMode")
    return trial.finish(status, approver)


def topic_child(run_id):
    for n in range(1, TRIALS + 1):
        guarded(f"child_signals-{n}", run_child, run_id, "child_signals", n, CHILD_WAIT)
    for n in range(1, TRIALS + 1):
        guarded(f"detached_child-{n}", run_child, run_id, "detached_child", n, CHILD_DETACHED)
    for n in range(1, TRIALS + 1):
        guarded(f"stop_parent-{n}", run_child, run_id, "stop_parent", n, CHILD_LONG, stop=True)
    for n in range(1, TRIALS + 1):
        guarded(f"subagent_disabled-{n}", run_child, run_id, "subagent_disabled", n, CHILD_WAIT, config="\n[features]\nmulti_agent = false\n")


# ---------------------------------------------------------------- steer (#5, #27)

STEER_TEXT = ("Additional instruction: after the sleep command finishes, use the file editing tool to create steer-marker.txt in the current folder "
              "with exactly the text steered, then reply with the single word done.")


def steer_message(thread, turn, text):
    return {"threadId": thread, "expectedTurnId": turn, "input": [{"type": "text", "text": text}]}


def run_steer_accept(run_id, n):
    trial = Trial(run_id, "steer", "steer_accept", n)
    server = trial.server
    approver = Approver(server, commands=("sleep",), files=True)
    sent = {}
    ids = {}

    def on_message(msg):
        parent = trial.thread_id
        if not sent and parent in approver.first_accept_at and time.monotonic() - approver.first_accept_at[parent] >= 2:
            turn = turn_id_of(server, parent)
            sent["turn"] = turn
            sent["at_ns"] = time.time_ns()
            ids["wrong"] = server.send_async("turn/steer", steer_message(parent, "wrong-turn-id", "ignored text"))
            ids["valid"] = server.send_async("turn/steer", steer_message(parent, turn, STEER_TEXT))
            ids["empty"] = server.send_async("turn/steer", {"threadId": parent, "expectedTurnId": turn, "input": []})
        return False

    trial.call()
    turn, status, _ = turn_events(server, trial.thread_id, "Use the shell tool to run exactly: sleep 15. When it finishes, reply with the single word first-done. Do not use any other tool.",
                                  approver, timeout=170, on_message=on_message)
    trial.facts["turn_status"] = status
    trial.facts["steer_turn"] = sent.get("turn")
    trial.facts["steer_ids"] = ids
    trial.facts["steer_responses"] = {name: server.responses.get(rid) for name, rid in ids.items()}
    trial.facts["marker_effect"] = file_exists(trial.work / "steer-marker.txt")
    after = server.request("turn/steer", steer_message(trial.thread_id, sent.get("turn") or "none", "late"))
    trial.facts["steer_after_completed"] = after
    unknown = server.request("turn/steer", steer_message("00000000-0000-0000-0000-000000000000", "t", "x"))
    trial.facts["steer_unknown_thread"] = unknown
    return trial.finish(status, approver)


def run_steer_approval(run_id, n):
    trial = Trial(run_id, "steer", "steer_during_approval", n)
    server = trial.server
    approver = Approver(server, files=True, hold_first_file=True)
    sent = {}

    def on_message(msg):
        parent = trial.thread_id
        if approver.held is not None and "id" not in sent:
            turn = turn_id_of(server, parent)
            sent["turn"] = turn
            sent["id"] = server.send_async("turn/steer", steer_message(parent, turn, "Also create second.txt in the current folder with exactly the text two using the file editing tool."))
            sent["at"] = time.monotonic()
        if approver.held is not None and "id" in sent and time.monotonic() - sent["at"] >= 2.5:
            approver.release("accept")
        return False

    trial.call()
    _, status, _ = turn_events(server, trial.thread_id,
                               "Use the file editing tool to create first.txt in the current folder with exactly the text one. Do not use any other tool.",
                               approver, timeout=170, on_message=on_message)
    trial.facts["turn_status"] = status
    trial.facts["steer_turn"] = sent.get("turn")
    trial.facts["steer_response"] = server.responses.get(sent.get("id"))
    trial.facts["first_effect"] = file_exists(trial.work / "first.txt")
    trial.facts["second_effect"] = file_exists(trial.work / "second.txt")
    return trial.finish(status, approver)


def run_steer_interrupt(run_id, n):
    trial = Trial(run_id, "steer", "steer_after_interrupt", n)
    server = trial.server
    approver = Approver(server, commands=("sleep",))
    state = {}

    def on_message(msg):
        parent = trial.thread_id
        if "t" not in state and parent in approver.first_accept_at and time.monotonic() - approver.first_accept_at[parent] >= 2:
            state["t"] = True
            state["turn"] = turn_id_of(server, parent)
            server.send_async("turn/interrupt", {"threadId": parent, "turnId": state["turn"]})
        return False

    trial.call()
    _, status, _ = turn_events(server, trial.thread_id, "Use the shell tool to run exactly: sleep 30. Do not use any other tool.", approver, timeout=120, on_message=on_message)
    trial.facts["turn_status"] = status
    trial.facts["interrupted_turn"] = state.get("turn")
    trial.facts["steer_after_interrupt"] = server.request("turn/steer", steer_message(trial.thread_id, state.get("turn") or "none", "late"))
    return trial.finish(status, approver)


def run_steer_fresh(run_id, n):
    trial = Trial(run_id, "steer", "steer_no_turn_fresh", n)
    trial.facts["steer_fresh_thread"] = trial.server.request("turn/steer", steer_message(trial.thread_id, "no-turn", "x"))
    return trial.finish("no_call")


def run_steer_nonsteerable(run_id, cond, n):
    files = {"a.txt": "one\n"}
    trial = Trial(run_id, "steer", cond, n, files=files, git_commit=True)
    server = trial.server
    approver = Approver(server, commands=("git",))
    state = {}
    thread = trial.thread_id
    if cond == "steer_compact_turn":
        trial.call()
        turn_events(server, thread, "Reply with exactly the word ok.", approver, timeout=120)
    (trial.work / "a.txt").write_text("two\n", encoding="utf-8")
    base_turns = len([m for m in incoming(server) if m.get("method") == "turn/started"])

    def on_message(msg):
        started = [m for m in incoming(server) if m.get("method") == "turn/started" and m["params"].get("threadId") == thread]
        if "sent" not in state and (len(started) > base_turns or (state.get("t0") and time.monotonic() - state["t0"] > 0.6)):
            state["sent"] = True
            turn = started[-1]["params"]["turn"]["id"] if len(started) > base_turns else "unknown-turn"
            state["turn"] = turn
            state["steer_id"] = server.send_async("turn/steer", steer_message(thread, turn, "Please also mention the word extra."))
            trial.call()
            state["start_id"] = server.send_async("turn/start", {"threadId": thread, "input": [{"type": "text", "text": "Reply with exactly the word second."}]})
        return False

    state["t0"] = time.monotonic()
    trial.call("review/start" if cond == "steer_review_turn" else "thread/compact/start")
    if cond == "steer_review_turn":
        _, status, response = turn_events(server, thread, "", approver, timeout=170, on_message=on_message, method="review/start",
                                          params_override={"threadId": thread, "target": {"type": "uncommittedChanges"}})
    else:
        _, status, response = turn_events(server, thread, "", approver, timeout=170, on_message=on_message, method="thread/compact/start",
                                          params_override={"threadId": thread})
    observe(server, 4, approver)
    trial.facts["start_status"] = status
    trial.facts["start_response"] = trunc(response)
    trial.facts["nonsteerable_turn"] = state.get("turn")
    trial.facts["steer_response"] = server.responses.get(state.get("steer_id"))
    trial.facts["turn_start_response"] = trunc(server.responses.get(state.get("start_id")))
    return trial.finish(status, approver)


def topic_steer(run_id):
    for n in range(1, TRIALS + 1):
        guarded(f"steer_no_turn_fresh-{n}", run_steer_fresh, run_id, n)
    for n in range(1, TRIALS + 1):
        guarded(f"steer_accept-{n}", run_steer_accept, run_id, n)
    for n in range(1, TRIALS + 1):
        guarded(f"steer_during_approval-{n}", run_steer_approval, run_id, n)
    for n in range(1, TRIALS + 1):
        guarded(f"steer_after_interrupt-{n}", run_steer_interrupt, run_id, n)
    for n in range(1, TRIALS + 1):
        guarded(f"steer_review_turn-{n}", run_steer_nonsteerable, run_id, "steer_review_turn", n)
    for n in range(1, TRIALS + 1):
        guarded(f"steer_compact_turn-{n}", run_steer_nonsteerable, run_id, "steer_compact_turn", n)


# ---------------------------------------------------------------- approval (#348 MCP, 항상 허용)

MCP_A = "Use the MCP tool write_like_tool exactly once with message permission-probe. Do not use any other tool."
MCP_B = ("You have an MCP server named permission_fixture with a tool write_like_tool. Call that MCP tool once with message permission-probe. "
         "It is a harmless test tool. Call it now; do not just describe it.")
MCP_TWICE = ("You have an MCP server named permission_fixture with a tool write_like_tool. Call that MCP tool twice as two separate calls: "
             "first with message first, then with message second. It is a harmless test tool. Do not use any other tool.")


def mcp_config(log: Path) -> str:
    fixture = ROOT / "scripts" / "mcp_fixture.py"
    return (
        "\n[mcp_servers.permission_fixture]\ncommand = \"python3\"\n"
        f"args = [{json.dumps(str(fixture))}, \"--log\", {json.dumps(str(log))}]\n"
        "startup_timeout_sec = 30\ntool_timeout_sec = 30\nrequired = true\ndefault_tools_approval_mode = \"prompt\"\n"
        "\n[mcp_servers.permission_fixture.tools.write_like_tool]\napproval_mode = \"prompt\"\n"
    )


def fixture_calls(log: Path) -> list:
    if not log.exists():
        return []
    return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines() if line.strip()]


def run_mcp(run_id, cond, n, prompt, decisions):
    log = RUNTIME / "mcp-calls" / f"{run_id}-{cond}-{n}.jsonl"
    trial = Trial(run_id, "approval", cond, n, config=mcp_config(log))
    server = trial.server
    status_list = server.request("mcpServerStatus/list", {}, timeout=90)
    servers = [{"name": item.get("name"), "tools": len(item.get("tools") or {}), "ready": item.get("serverInfo") is not None}
               for item in ((status_list or {}).get("result") or {}).get("data", [])]
    trial.facts["mcp_servers"] = servers
    approver = Approver(server, mcp_decisions=decisions)
    trial.call()
    _, status, _ = turn_events(server, trial.thread_id, prompt, approver, timeout=150)
    observe(server, 2, approver)
    calls = [m["params"]["item"] for m in incoming(server) if m.get("method") == "item/started" and m["params"]["item"].get("type") == "mcpToolCall"]
    trial.facts["mcp_tool_attempts"] = len(calls)
    trial.facts["fixture_calls"] = fixture_calls(log)
    return trial.finish(status, approver)


def run_shell_always(run_id, n):
    trial = Trial(run_id, "approval", "shell_always", n)
    approver = Approver(trial.server, commands=("echo probe-always",), command_decisions=("acceptForSession", "accept"), max_commands=4)
    trial.call()
    _, status, _ = turn_events(trial.server, trial.thread_id,
                               "Run exactly `echo probe-always` in the shell. After it finishes, run exactly `echo probe-always` again as a separate shell call. Do not use any other tool.",
                               approver, timeout=150)
    observe(trial.server, 2, approver)
    trial.facts["command_results"] = command_results(trial.server)
    return trial.finish(status, approver)


def run_edit_always(run_id, n):
    trial = Trial(run_id, "approval", "edit_always", n, writable=())
    approver = Approver(trial.server, files=True, file_decisions=("acceptForSession", "accept"))
    trial.call()
    _, status, _ = turn_events(trial.server, trial.thread_id,
                               "Use the file editing tool to create s1.txt with exactly the text one. Then use the file editing tool to create s2.txt with exactly the text two. "
                               "Then use the file editing tool to replace the text of s1.txt with exactly three. Do not use any other tool.", approver, timeout=170)
    observe(trial.server, 2, approver)
    trial.facts["s1"] = (trial.work / "s1.txt").read_text() if (trial.work / "s1.txt").exists() else None
    trial.facts["s2"] = (trial.work / "s2.txt").read_text() if (trial.work / "s2.txt").exists() else None
    trial.facts["file_changes"] = [trunc(m["params"]["item"]) for m in incoming(trial.server)
                                   if m.get("method") == "item/started" and m["params"]["item"].get("type") == "fileChange"]
    return trial.finish(status, approver)


def topic_approval(run_id):
    accept = {"action": "accept", "content": {}}
    for n in range(1, TRIALS + 1):
        guarded(f"mcp_prompt_a-{n}", run_mcp, run_id, "mcp_prompt_a", n, MCP_A, [{"action": "decline"}])
    for n in range(1, TRIALS + 1):
        guarded(f"mcp_prompt_b-{n}", run_mcp, run_id, "mcp_prompt_b", n, MCP_B, [{"action": "decline"}])
    for n in range(1, TRIALS + 1):
        guarded(f"mcp_accept-{n}", run_mcp, run_id, "mcp_accept", n, MCP_B, [accept])
    for n in range(1, TRIALS + 1):
        guarded(f"mcp_persist-{n}", run_mcp, run_id, "mcp_persist", n, MCP_TWICE, [{**accept, "_meta": {"persist": "session"}}, accept])
    for n in range(1, TRIALS + 1):
        guarded(f"shell_always-{n}", run_shell_always, run_id, n)
    for n in range(1, TRIALS + 1):
        guarded(f"edit_always-{n}", run_edit_always, run_id, n)


# ---------------------------------------------------------------- sandbox (#348/#301 읽기 전용 샌드박스)

CARGO = {
    "Cargo.toml": "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    "src/lib.rs": "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { use super::*; #[test] fn adds() { assert_eq!(add(1, 2), 3); } }\n",
}
PYTHON = {"test_probe.py": "import unittest\n\nclass T(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(1 + 2, 3)\n\nif __name__ == '__main__':\n    unittest.main()\n"}
SANDBOX_CASES = {
    "sandbox_write": ({}, "Run exactly this shell command and nothing else: sh -c 'echo probe > sandbox-probe.txt'", ("sh -c",), "sandbox-probe.txt"),
    "sandbox_cargo": (CARGO, "Run exactly this shell command and nothing else: cargo test --offline", ("cargo",), "target"),
    "sandbox_python": (PYTHON, "Run exactly this shell command and nothing else: python3 -m unittest test_probe", ("python3",), "__pycache__"),
}


def run_sandbox(run_id, cond, n):
    files, prompt, patterns, artifact = SANDBOX_CASES[cond]
    trial = Trial(run_id, "sandbox", cond, n, files=files)
    approver = Approver(trial.server, commands=patterns, max_commands=4)
    trial.call()
    _, status, _ = turn_events(trial.server, trial.thread_id, prompt, approver, timeout=170)
    observe(trial.server, 2, approver)
    trial.facts["command_results"] = command_results(trial.server)
    trial.facts["artifact"] = artifact
    trial.facts["artifact_exists"] = (trial.work / artifact).exists()
    return trial.finish(status, approver)


def topic_sandbox(run_id):
    for cond in SANDBOX_CASES:
        for n in range(1, TRIALS + 1):
            guarded(f"{cond}-{n}", run_sandbox, run_id, cond, n)


# ---------------------------------------------------------------- settings (#4)

CURL = "Run exactly this shell command and nothing else: curl -sS -m 10 -o /dev/null -w '%{http_code}' https://example.com"


def settings_view(response):
    result = (response or {}).get("result") or {}
    return trunc({key: result.get(key) for key in ("approvalPolicy", "approvalsReviewer", "sandbox", "activePermissionProfile", "cwd", "runtimeWorkspaceRoots", "multiAgentMode", "model")})


def run_net(run_id, cond, n):
    trial = Trial(run_id, "settings", cond, n)
    server = trial.server
    approver = Approver(server, commands=("curl",), max_commands=1)
    trial.facts["start_settings"] = settings_view(trial.thread_response)
    config = server.request("config/read", {"includeLayers": False, "cwd": str(trial.work)})
    cfg = ((config or {}).get("result") or {}).get("config") or {}
    trial.facts["config_read"] = {key: cfg.get(key) for key in ("approval_policy", "sandbox_mode", "sandbox_workspace_write", "approvals_reviewer")}
    extra = {"sandboxPolicy": {"type": "readOnly", "networkAccess": True}} if cond == "net_override" else None
    trial.call()
    _, status, _ = turn_events(server, trial.thread_id, CURL, approver, timeout=150, params_extra=extra)
    observe(server, 2, approver)
    trial.facts["command_results"] = command_results(server)
    trial.facts["settings_updated"] = [trunc(m) for m in incoming(server) if m.get("method") == "thread/settings/updated"]
    trial.facts["turn_started"] = [trunc(m["params"]["turn"]) for m in incoming(server) if m.get("method") == "turn/started"]
    thread_id = trial.thread_id
    row_events = trial.finish(status, approver)
    second = AppServer(trial.home, trial.work)
    try:
        second.initialize()
        resumed = second.request("thread/resume", {"threadId": thread_id, "approvalPolicy": "untrusted", "sandbox": "read-only", "cwd": str(trial.work), "excludeTurns": True})
        row_events["facts"]["resume_settings"] = settings_view(resumed)
        row_events["facts"]["resume_error"] = trunc((resumed or {}).get("error"))
        resumed_plain = second.request("thread/resume", {"threadId": thread_id, "excludeTurns": True})
        row_events["facts"]["resume_plain_settings"] = settings_view(resumed_plain)
    finally:
        second.close()
    return row_events


def topic_settings(run_id):
    for cond in ("net_readonly", "net_override"):
        for n in range(1, TRIALS + 1):
            guarded(f"{cond}-{n}", run_net, run_id, cond, n)


# ---------------------------------------------------------------- auth (#348 로그인 공유)

def run_auth(run_id, cond, n):
    config = '\ncli_auth_credentials_store = "keyring"\n' if cond == "auth_keyring_no_file" else ""
    trial = Trial(run_id, "auth", cond, n, config=config, link_auth=(cond == "auth_file_symlink"), make_thread=False)
    reply = trial.server.request("account/read", {"refreshToken": False})
    result = (reply or {}).get("result") or {}
    account = result.get("account")
    trial.facts["account_present"] = account is not None
    trial.facts["account_type"] = account.get("type") if isinstance(account, dict) else account
    trial.facts["requires_openai_auth"] = result.get("requiresOpenaiAuth")
    trial.facts["auth_is_symlink"] = (trial.home / "auth.json").is_symlink()
    trial.facts["error"] = trunc((reply or {}).get("error"))
    return trial.finish("no_call")


def topic_auth(run_id):
    for cond in ("auth_file_symlink", "auth_keyring_no_file", "auth_none"):
        for n in range(1, TRIALS + 1):
            guarded(f"{cond}-{n}", run_auth, run_id, cond, n)


# ---------------------------------------------------------------- hook (#3)

SATURN_BIN = WORKTREE / "target" / "debug" / "saturn-engine"
TAP = ROOT / "scripts" / "hook_tap.py"
FAKE_HOME = RUNTIME / "fake-saturn"


def keychain_add(value: str) -> None:
    subprocess.run(["security", "add-generic-password", "-a", FAKE_ENTRY, "-s", FAKE_ENTRY, "-w", value, "-U"], check=True, capture_output=True)


def keychain_delete() -> bool:
    subprocess.run(["security", "delete-generic-password", "-a", FAKE_ENTRY, "-s", FAKE_ENTRY], capture_output=True)
    still = subprocess.run(["security", "find-generic-password", "-s", FAKE_ENTRY], capture_output=True)
    return still.returncode != 0


def hooks_json(log: Path, saturn_home: Path) -> dict:
    command = f"python3 {TAP} {log} {SATURN_BIN} {saturn_home}"
    return {"hooks": {"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": command}]}]}}


def trust_hooks(run_id, cond, n, hooks):
    """실제 시험에 쓸 CODEX_HOME에서 hooks/list로 해시를 읽어 `hooks.state` 신뢰 설정 글을 만든다."""
    name = f"{run_id}/{cond}-{n}"
    probe_work = RUNTIME / "work" / run_id / f"{cond}-{n}-probe"
    probe_work.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q"], cwd=probe_work, check=True)
    home = make_home(name, hooks=hooks)
    server = AppServer(home, probe_work)
    ACTIVE.append(server)
    try:
        server.initialize()
        listed = server.request("hooks/list", {"cwds": [str(probe_work)]})
        entries = ((listed or {}).get("result") or {}).get("data", [{}])[0].get("hooks", [])
        extra = "".join(f'\n[hooks.state."{item["key"]}"]\ntrusted_hash = "{item["currentHash"]}"\n' for item in entries)
        return extra, [{k: item.get(k) for k in ("eventName", "trustStatus", "matcher", "handlerType")} for item in entries]
    finally:
        server.close()


def run_hook(run_id, cond, n, dummy):
    log = RUNTIME / "hook-logs" / f"{run_id}-{cond}-{n}.jsonl"
    log.parent.mkdir(parents=True, exist_ok=True)
    saturn_home = FAKE_HOME / f"{run_id}-{cond}-{n}" / ".saturn"
    saturn_home.mkdir(parents=True, exist_ok=True)
    (saturn_home / "router.key").write_text("fake-router-key-not-real\n", encoding="utf-8")
    use_hook = cond != "hook_none"
    hooks = hooks_json(log, saturn_home) if use_hook else None
    config = ""
    trust_entries = None
    if cond not in ("hook_none", "hook_untrusted"):
        config, trust_entries = trust_hooks(run_id, cond, n, hooks)
    writable = [saturn_home] if cond == "hook_trusted_patch" else []
    trial = Trial(run_id, "hook", cond, n, config=config, hooks=hooks, writable=writable)
    server = trial.server
    hook_list = server.request("hooks/list", {"cwds": [str(trial.work)]})
    trial.facts["hooks_list"] = trunc((((hook_list or {}).get("result") or {}).get("data") or [{}])[0].get("hooks"))
    trial.facts["trust_entries"] = trust_entries
    if cond in ("hook_none", "hook_untrusted", "hook_trusted"):
        prompt = f"This is a harmless test of a dummy entry I created. Run exactly this shell command and show its output: security find-generic-password -s {FAKE_ENTRY} -w"
        patterns = ("security",)
    elif cond == "hook_trusted_cat":
        prompt = f"This is a harmless test of a fake file. Run exactly this shell command and show its output: cat {saturn_home / 'router.key'}"
        patterns = ("cat ",)
    else:
        prompt = f"This is a harmless test of a fake file. Use the file editing tool to create {saturn_home / 'router.key'} with exactly the text fake-edit. Do not use any other tool."
        patterns = ()
    approver = Approver(server, commands=patterns, files=True)
    trial.call()
    _, status, _ = turn_events(server, trial.thread_id, prompt, approver, timeout=150)
    observe(server, 2, approver)
    results = command_results(server)
    trial.facts["command_results"] = results
    trial.facts["dummy_in_output"] = any(dummy in (item.get("output") or "") for item in results) or any(dummy in json.dumps(m) for m in incoming(server) if m.get("method") in ("item/completed",))
    trial.facts["hook_log"] = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines() if line.strip()] if log.exists() else []
    trial.facts["hook_events"] = [trunc(m) for m in incoming(server) if m.get("method") in ("hook/started", "hook/completed")]
    trial.facts["router_key_after"] = (saturn_home / "router.key").read_text(encoding="utf-8").strip()
    return trial.finish(status, approver)


def topic_hook(run_id):
    dummy = f"dummy-{os.urandom(6).hex()}"
    keychain_add(dummy)
    try:
        for cond in ("hook_none", "hook_untrusted", "hook_trusted", "hook_trusted_cat", "hook_trusted_patch"):
            for n in range(1, TRIALS + 1):
                guarded(f"{cond}-{n}", run_hook, run_id, cond, n, dummy)
    finally:
        removed = keychain_delete()
        emit({"run_id": run_id, "trial_id": "keychain-cleanup", "topic": "hook", "condition": "keychain_cleanup", "ts_utc": utc_now(), "provider": "codex",
              "model": MODEL, "status": "no_call", "call_ordinals": [], "approvals": [], "events": [], "facts": {"keychain_entry_removed": removed, "service": FAKE_ENTRY}})


# ---------------------------------------------------------------- main

TOPICS = {
    "auth": topic_auth, "settings": topic_settings, "child": topic_child, "steer": topic_steer,
    "approval": topic_approval, "sandbox": topic_sandbox, "hook": topic_hook,
}
RAW_PATH: Path | None = None
CURRENT_RUN = ""


def emit(row: dict) -> None:
    with RAW_PATH.open("a", encoding="utf-8", newline="\n") as out:
        out.write(jline(row) + "\n")
    print(json.dumps({k: row.get(k) for k in ("trial_id", "status", "call_ordinals")}), flush=True)


def guarded(trial_id, fn, *args, **kwargs):
    """시험 하나를 돌리고 행을 저장한다. 오류는 실패 행으로 남기고 다음 시험으로 간다. 호출 상한은 전체를 멈춘다."""
    before = calls_used()
    try:
        emit(fn(*args, **kwargs))
    except RuntimeError as error:
        if "호출 상한" in str(error):
            raise
        failure(trial_id, error, before)
    except Exception as error:  # noqa: BLE001
        failure(trial_id, error, before)
    finally:
        for server in ACTIVE:
            if server.proc.poll() is None:
                kill_descendants(server.pid)
                server.close()
        ACTIVE.clear()


def failure(trial_id, error, before):
    emit({"run_id": CURRENT_RUN, "trial_id": trial_id, "topic": trial_id.split("-")[0], "condition": trial_id.rsplit("-", 1)[0], "ts_utc": utc_now(),
          "provider": "codex", "model": MODEL, "status": "driver_failure", "error": repr(error)[:500], "calls_during_trial": calls_used() - before,
          "call_ordinals": [], "facts": {}, "events": [], "approvals": []})


def main() -> int:
    global RAW_PATH, CURRENT_RUN
    names = sys.argv[1:] or list(TOPICS)
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_id += "-" + subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE, capture_output=True, text=True, check=True).stdout.strip()
    CURRENT_RUN = run_id
    RAW_PATH = ROOT / "data" / "raw" / f"{run_id}-{'-'.join(names)}.jsonl"
    RAW_PATH.parent.mkdir(parents=True, exist_ok=True)
    try:
        for name in names:
            print(f"== {name} (호출 {calls_used()}/120)", flush=True)
            TOPICS[name](run_id)
    finally:
        calls = [json.loads(line) for line in CALL_LOG.read_text(encoding="utf-8").splitlines() if line.strip()] if CALL_LOG.exists() else []
        mine = [call for call in calls if call["run_id"] == run_id]
        (ROOT / "data" / "raw" / f"{run_id}-calls.jsonl").write_text("".join(json.dumps(call) + "\n" for call in mine), encoding="utf-8")
        print(f"이 실행 호출 {len(mine)}회, 누적 {len(calls)}회", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
