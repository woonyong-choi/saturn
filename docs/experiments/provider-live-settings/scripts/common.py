from __future__ import annotations

import datetime as dt
import json
import os
import selectors
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]
MAIN_REPO = Path.home() / "workspace" / "oss" / "saturn"
PRIVATE = MAIN_REPO / ".local" / "experiments" / "provider-live-settings"
CLAUDE_MODEL = "claude-haiku-4-5-20251001"
CODEX_MODEL = "gpt-5.6-luna"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def redact(value):
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    if not isinstance(value, str):
        return value
    value = value.replace(str(Path.home()), "~")
    for name in ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "SATURN_JUDGE_KEY"):
        secret = os.environ.get(name)
        if secret:
            value = value.replace(secret, "[secret]")
    return value


def private_log(name: str, events: list[dict]) -> str:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    path = PRIVATE / name
    with path.open("w", encoding="utf-8", newline="\n") as stream:
        for event in events:
            stream.write(json.dumps(redact(event), ensure_ascii=False, separators=(",", ":")) + "\n")
    return str(path.relative_to(MAIN_REPO))


class LineProcess:
    def __init__(self, args: list[str], env: dict[str, str] | None = None):
        self.proc = subprocess.Popen(args, cwd=WORKTREE, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     start_new_session=True)
        self.events: list[dict] = []
        self.buffer = bytearray()
        os.set_blocking(self.proc.stdout.fileno(), False)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.proc.stdout.fileno(), selectors.EVENT_READ)

    @property
    def pid(self) -> int:
        return self.proc.pid

    def send(self, value: dict) -> None:
        data = (json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        try:
            self.proc.stdin.write(data)
            self.proc.stdin.flush()
        except BrokenPipeError:
            return
        self.events.append({"direction": "out", "ts_ns": time.time_ns(), "message": value})

    def pump(self, timeout: float = 0.25) -> list[dict]:
        messages = []
        ready = self.selector.select(timeout)
        if not ready:
            return messages
        while True:
            try:
                chunk = os.read(self.proc.stdout.fileno(), 65536)
            except BlockingIOError:
                break
            if not chunk:
                break
            received = time.time_ns()
            self.buffer.extend(chunk)
            while b"\n" in self.buffer:
                line, _, rest = self.buffer.partition(b"\n")
                self.buffer = bytearray(rest)
                text = line.decode("utf-8", errors="replace")
                try:
                    message = json.loads(text)
                except json.JSONDecodeError:
                    message = {"type": "non_json", "text": text}
                event = {"direction": "in", "rawByteReceivedAtNs": received, "message": message}
                self.events.append(event)
                messages.append(message)
            if len(chunk) < 65536:
                break
        return messages

    def wait_for(self, predicate, timeout: float = 60.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(min(0.25, max(0.0, deadline - time.monotonic()))):
                if predicate(message):
                    return message
            if self.proc.poll() is not None and not self.buffer:
                break
        return None

    def close(self) -> str:
        self.selector.close()
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
                self.proc.wait(timeout=3)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        try:
            stderr = self.proc.stderr.read().decode("utf-8", errors="replace")
        except OSError:
            stderr = ""
        return redact(stderr)


class ClaudeDriver(LineProcess):
    def __init__(self, trial: str, permission_mode: str = "plan"):
        args = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
                "--verbose", "--no-session-persistence", "--strict-mcp-config", "--session-id", trial,
                "--model", CLAUDE_MODEL, "--permission-prompt-tool", "stdio",
                "--permission-mode", permission_mode, "--tools", "Bash", "AskUserQuestion"]
        super().__init__(args)

    def init(self, timeout: float = 60.0):
        return self.wait_for(lambda m: m.get("type") == "system" and m.get("subtype") == "init", timeout)

    def user(self, text: str):
        self.send({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}})

    def control(self, request_id: str, request: dict, timeout: float = 10.0):
        self.send({"type": "control_request", "request_id": request_id, "request": request})
        return self.wait_for(lambda m: m.get("type") == "control_response"
                             and m.get("response", {}).get("request_id") == request_id, timeout)

    def finish_turn(self, permission: str = "allow", ask_user: str | None = None,
                    timeout: float = 45.0):
        decisions = []
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if message.get("type") == "control_request":
                    request = message.get("request", {})
                    tool = request.get("tool_name")
                    behavior = ask_user if tool == "AskUserQuestion" and ask_user else permission
                    response = {"behavior": behavior}
                    if behavior == "allow":
                        response["updatedInput"] = request.get("input", {})
                    self.send({"type": "control_response", "response": {
                        "subtype": "success", "request_id": message.get("request_id"), "response": response,
                    }})
                    decisions.append({"tool": tool, "behavior": behavior, "request_id": message.get("request_id")})
                if message.get("type") == "result":
                    return decisions, message
            if self.proc.poll() is not None:
                break
        return decisions, None


class CodexDriver(LineProcess):
    def __init__(self, home: Path):
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        super().__init__(["codex", "app-server"], env)
        self.next_id = 1
        self.server_requests: list[dict] = []

    def request(self, method: str, params=None, timeout: float = 30.0, approval: str = "decline"):
        request_id = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if "method" in message and "id" in message:
                    self.server_requests.append(message)
                    self.answer_server_request(message, approval)
                if message.get("id") == request_id:
                    return message
            if self.proc.poll() is not None:
                break
        return None

    def answer_server_request(self, message: dict, approval: str):
        method = message.get("method", "")
        if method.endswith("requestApproval") or method in {"execCommandApproval", "applyPatchApproval"}:
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": {"decision": approval}})
        else:
            self.send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "unhandled experiment request"}})

    def initialize(self):
        result = self.request("initialize", {"clientInfo": {"name": "provider-live-settings", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "initialized"})
        return result

    def start_thread(self, *, approval_policy=None, sandbox=None):
        params = {"cwd": str(WORKTREE), "model": CODEX_MODEL, "ephemeral": True}
        if approval_policy is not None:
            params["approvalPolicy"] = approval_policy
        if sandbox is not None:
            params["sandbox"] = sandbox
        return self.request("thread/start", params)

    def turn(self, thread_id: str, text: str, overrides=None, approval: str = "decline"):
        params = {"threadId": thread_id, "input": [{"type": "text", "text": text}]}
        if overrides:
            params.update(overrides)
        started = self.request("turn/start", params, approval=approval)
        events = []
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                events.append(message)
                if "method" in message and "id" in message:
                    self.server_requests.append(message)
                    self.answer_server_request(message, approval)
                if message.get("method") == "turn/completed" and message.get("params", {}).get("threadId") == thread_id:
                    return started, events
            if self.proc.poll() is not None:
                break
        return started, events


def setup_codex_home(name: str, config: str, rules: str) -> Path:
    home = WORKTREE / ".runtime" / "codex-homes" / name
    (home / "rules").mkdir(parents=True, exist_ok=True)
    (home / "config.toml").write_text(config, encoding="utf-8")
    (home / "rules" / "default.rules").write_text(rules, encoding="utf-8")
    auth = home / "auth.json"
    source = Path.home() / ".codex" / "auth.json"
    if not auth.exists() and source.exists():
        auth.symlink_to(source)
    return home


def marker_path(trial: str, phase: str) -> Path:
    path = WORKTREE / ".runtime" / "markers" / f"{trial}-{phase}.txt"
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        path.unlink()
    return path


def marker_prompt(path: Path, label: str) -> str:
    relative = path.relative_to(WORKTREE)
    return f"Use the shell tool and run exactly: printf '{label}' > {relative}. Do not do anything else."
