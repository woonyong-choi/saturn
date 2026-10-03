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
PRIVATE = MAIN_REPO / ".local" / "experiments" / "codex-live-reload"
CODEX_MODEL = "gpt-5.6-luna"
MAX_TURN_CALLS = 50


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def redact(value):
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            lowered = str(key).lower()
            if any(word in lowered for word in ("token", "secret", "authorization", "api_key", "apikey", "password")):
                result[key] = "[redacted]"
            else:
                result[key] = redact(item)
        return result
    if isinstance(value, list):
        return [redact(item) for item in value]
    if not isinstance(value, str):
        return value
    result = value.replace(str(Path.home()), "~")
    for name in ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "SATURN_JUDGE_KEY"):
        secret = os.environ.get(name)
        if secret:
            result = result.replace(secret, "[secret]")
    return result


def private_log(name: str, events: list[dict]) -> str:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    path = PRIVATE / name
    with path.open("w", encoding="utf-8", newline="\n") as stream:
        for event in events:
            stream.write(json.dumps(redact(event), ensure_ascii=False, separators=(",", ":")) + "\n")
    return str(path.relative_to(MAIN_REPO))


class LineProcess:
    def __init__(self, args: list[str], env: dict[str, str] | None = None):
        self.proc = subprocess.Popen(
            args,
            cwd=WORKTREE,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
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
        except (BrokenPipeError, OSError):
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


class CodexDriver(LineProcess):
    def __init__(self, home: Path):
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        super().__init__(["codex", "app-server"], env)
        self.next_id = 1
        self.server_requests: list[dict] = []
        self.turn_events: list[list[dict]] = []

    def request(self, method: str, params=None, timeout: float = 30.0, approval: str = "decline"):
        request_id = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if "method" in message and "id" in message:
                    self.handle_server_request(message, approval)
                if message.get("id") == request_id:
                    return message
            if self.proc.poll() is not None:
                break
        return None

    def handle_server_request(self, message: dict, approval: str = "decline") -> None:
        self.server_requests.append(message)
        method = message.get("method", "")
        if method == "item/tool/requestUserInput":
            questions = message.get("params", {}).get("questions", [])
            answers = {}
            for question in questions:
                options = question.get("options", [])
                answers[question.get("id", "question")] = options[0].get("label", "실험 답변") if options else "실험 답변"
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": {"answers": answers}})
        elif method.endswith("requestApproval") or method in {"execCommandApproval", "applyPatchApproval"}:
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": {"decision": approval}})

    def initialize(self):
        result = self.request("initialize", {"clientInfo": {"name": "codex-live-reload", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "initialized"})
        return result

    def start_thread(self):
        params = {"cwd": str(WORKTREE), "model": CODEX_MODEL, "ephemeral": True}
        return self.request("thread/start", params)

    def turn(self, thread_id: str, text: str, approval: str = "decline"):
        params = {"threadId": thread_id, "input": [{"type": "text", "text": text}]}
        started = self.request("turn/start", params, approval=approval)
        events = []
        deadline = time.monotonic() + 75
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                events.append(message)
                if "method" in message and "id" in message:
                    self.handle_server_request(message, approval)
                if message.get("method") == "turn/completed" and message.get("params", {}).get("threadId") == thread_id:
                    self.turn_events.append(events)
                    return started, events
            if self.proc.poll() is not None:
                break
        self.turn_events.append(events)
        return started, events

    def model_calls(self) -> int:
        return sum(
            1
            for event in self.events
            if event.get("direction") == "in"
            and event.get("message", {}).get("method") == "rawResponse/completed"
            and "usage" in event.get("message", {}).get("params", {})
        )


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


def question_prompt() -> str:
    return "Ask the user one question with the request user input tool. Offer exactly two distinct choices, then report the selected answer."


def touch_prompt(path: Path) -> str:
    relative = path.relative_to(WORKTREE)
    return f"Use the shell tool and run exactly: touch {relative}. Do not do anything else."


def response_status(response) -> str:
    if response and "result" in response:
        return "success"
    if response and "error" in response:
        return "error"
    return "timeout"
