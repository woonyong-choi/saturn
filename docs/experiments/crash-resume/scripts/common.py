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
MAIN_REPO = Path("/Users/woonyong/workspace/oss/saturn")
PRIVATE = MAIN_REPO / ".local" / "experiments" / "crash-resume"
CODEX_MODEL = "gpt-5.6-luna"
CLAUDE_MODEL = "claude-haiku-4-5-20251001"
CODEX_CAP = 40
CLAUDE_CAP = 30
MARKER_WAIT_SECONDS = 90
POST_RESUME_WAIT_SECONDS = 35


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def redact(value):
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if any(word in str(key).lower() for word in ("token", "secret", "authorization", "password", "credential")):
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
    def __init__(self, args: list[str], env: dict[str, str]):
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
        if not self.selector.select(timeout):
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

    def kill_self(self) -> None:
        if self.proc.poll() is None:
            try:
                os.kill(self.proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass

    def close(self) -> str:
        self.selector.close()
        if self.proc.poll() is None:
            try:
                os.kill(self.proc.pid, signal.SIGTERM)
                self.proc.wait(timeout=3)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.kill(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        try:
            return redact(self.proc.stderr.read().decode("utf-8", errors="replace"))
        except OSError:
            return ""


def marker_paths(trial_id: str) -> tuple[Path, Path]:
    directory = WORKTREE / ".runtime" / "markers"
    directory.mkdir(parents=True, exist_ok=True)
    events = directory / f"{trial_id}.events"
    done = directory / f"{trial_id}.done"
    for path in (events, done):
        if path.exists() or path.is_symlink():
            path.unlink()
    return events, done


def marker_counts(events: Path, done: Path) -> dict[str, int]:
    counts = {"start": 0, "complete": 0, "touch": int(done.exists())}
    if events.exists():
        for line in events.read_text(encoding="utf-8").splitlines():
            kind = line.split(":", 1)[0]
            if kind in ("start", "complete"):
                counts[kind] += 1
    return counts


def marker_prompt(events: Path, done: Path, invocation_id: str, task_text: str) -> str:
    event_path = events.relative_to(WORKTREE)
    done_path = done.relative_to(WORKTREE)
    command = (
        f"sh -c 'printf \"start:{invocation_id}:$\\$\\n\" >> {event_path}; "
        f"sleep 30; printf \"complete:{invocation_id}:$\\$\\n\" >> {event_path}; touch {done_path}'"
    ).replace("$\\$", "$$")
    return f"{task_text} Use exactly this harmless command and do not do anything else: {command}"


def model_call_count(events: list[dict], provider: str) -> int:
    if provider == "codex":
        return sum(
            1
            for event in events
            if event.get("direction") == "in"
            and event.get("message", {}).get("method") == "rawResponse/completed"
        )
    return sum(
        1
        for event in events
        if event.get("direction") == "in"
        and event.get("message", {}).get("type") == "assistant"
    )


def response_status(response) -> str:
    if response and "result" in response:
        return "success"
    if response and "error" in response:
        return "error"
    return "timeout"


def setup_codex_home(trial_id: str) -> Path:
    home = WORKTREE / ".runtime" / "codex-homes" / trial_id
    (home / "rules").mkdir(parents=True, exist_ok=True)
    (home / "config.toml").write_text(
        'approval_policy = "never"\nsandbox_mode = "workspace-write"\n', encoding="utf-8"
    )
    (home / "rules" / "default.rules").write_text("", encoding="utf-8")
    source = Path.home() / ".codex" / "auth.json"
    link = home / "auth.json"
    if source.exists() and not link.exists():
        link.symlink_to(source)
    return home


def setup_claude_config(trial_id: str) -> Path:
    config = WORKTREE / ".runtime" / "claude-config" / trial_id
    config.mkdir(parents=True, exist_ok=True)
    source = Path.home() / ".claude" / ".credentials.json"
    link = config / ".credentials.json"
    if source.exists() and not link.exists():
        link.symlink_to(source)
    return config


class CodexDriver(LineProcess):
    def __init__(self, home: Path):
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        super().__init__(["codex", "app-server"], env)
        self.next_id = 1
        self.server_requests: list[dict] = []

    def request(self, method: str, params=None, timeout: float = 30.0):
        request_id = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if "method" in message and "id" in message:
                    self.server_requests.append(message)
                    self.answer_server_request(message)
                if message.get("id") == request_id:
                    return message
            if self.proc.poll() is not None:
                break
        return None

    def answer_server_request(self, message: dict) -> None:
        method = message.get("method", "")
        if method.endswith("requestApproval") or method in {"execCommandApproval", "applyPatchApproval"}:
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": {"decision": "accept"}})
        elif method == "item/tool/requestUserInput":
            questions = message.get("params", {}).get("questions", [])
            answers = {
                question.get("id", "question"): {"answers": [question.get("options", [{}])[0].get("label", "계속")]}
                for question in questions
            }
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": {"answers": answers}})

    def initialize(self):
        result = self.request("initialize", {"clientInfo": {"name": "crash-resume", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "initialized"})
        return result

    def start_thread(self):
        return self.request("thread/start", {
            "cwd": str(WORKTREE), "model": CODEX_MODEL, "ephemeral": False,
            "approvalPolicy": "never", "sandbox": "workspace-write",
        })


class ClaudeDriver(LineProcess):
    def __init__(self, session_id: str, config: Path, resume: bool, resume_env: bool, task: bool = False):
        env = os.environ.copy()
        env["CLAUDE_CONFIG_DIR"] = str(config)
        if resume_env:
            env["CLAUDE_CODE_RESUME_INTERRUPTED_TURN"] = "1"
        else:
            env.pop("CLAUDE_CODE_RESUME_INTERRUPTED_TURN", None)
        args = [
            "claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
            "--verbose", "--strict-mcp-config", "--setting-sources", "user",
            "--model", CLAUDE_MODEL, "--permission-prompt-tool", "stdio",
            "--permission-mode", "manual", "--tools", "Bash",
        ]
        if task:
            args.extend(["Task"])
        if resume:
            args.extend(["--resume", session_id])
        else:
            args.extend(["--session-id", session_id])
        super().__init__(args, env)

    def send_user(self, text: str) -> None:
        self.send({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}})

    def allow_control_request(self, message: dict) -> None:
        request = message.get("request", {})
        response = {"behavior": "allow", "updatedInput": request.get("input", {})}
        self.send({"type": "control_response", "response": {
            "subtype": "success", "request_id": message.get("request_id"), "response": response,
        }})

    def wait_initialised(self, timeout: float = 60.0) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if message.get("type") == "system" and message.get("subtype") == "init":
                    return True
            if self.proc.poll() is not None:
                break
        return False

    def observe(self, events_path: Path, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for message in self.pump(0.25):
                if message.get("type") == "control_request":
                    self.allow_control_request(message)


def event_summary(events: list[dict]) -> dict:
    methods = {}
    for event in events:
        message = event.get("message", {})
        name = message.get("method") or message.get("type") or "unknown"
        methods[name] = methods.get(name, 0) + 1
    return methods
