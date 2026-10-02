"""공통 수집기. provider 원문은 즉시 기록하고 절대 경로와 비밀값은 가린다."""
from __future__ import annotations

import json
import os
import re
import selectors
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]
MAIN_REPO = Path.home() / "workspace" / "oss" / "saturn"
RAW = ROOT / "data" / "raw"
PRIVATE = MAIN_REPO / ".local" / "experiments" / "provider-decision-checks"
CLAUDE_MODEL = "claude-haiku-4-5-20251001"
CODEX_MODEL = "gpt-5.6-luna"
TIMEOUT_S = 180
CAP = 80


def utc_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def redact(value):
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    if not isinstance(value, str):
        return value
    secrets = [os.environ.get("SATURN_JUDGE_KEY", ""), os.environ.get("OPENAI_API_KEY", "")]
    for secret in secrets:
        if secret:
            value = value.replace(secret, "[secret]")
    value = value.replace(str(Path.home()), "~")
    value = re.sub(r"(?i)(bearer\s+)[A-Za-z0-9._=-]+", r"\1[secret]", value)
    value = re.sub(r"(?i)(api[_-]?key|token|secret)\s*[:=]\s*[^,\s}\]]+", r"\1=[secret]", value)
    return value


def json_line(value) -> str:
    return json.dumps(redact(value), ensure_ascii=False, separators=(",", ":"))


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as out:
        for row in rows:
            out.write(json_line(row) + "\n")


def append_jsonl(path: Path, row: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8", newline="\n") as out:
        out.write(json_line(row) + "\n")


def save_private(trial_id: str, events: list, stdout: str = "", stderr: str = "") -> str:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    path = PRIVATE / f"{trial_id}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as out:
        for event in events:
            out.write(json_line(event) + "\n")
        if stdout:
            out.write(json_line({"kind": "stdout", "text": stdout}) + "\n")
        if stderr:
            out.write(json_line({"kind": "stderr", "text": stderr}) + "\n")
    return str(path).replace(str(MAIN_REPO) + "/", "")


def save_screen(trial_id: str, text: str) -> str:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    path = PRIVATE / f"{trial_id}.txt"
    path.write_text(redact(text), encoding="utf-8")
    return str(path).replace(str(MAIN_REPO) + "/", "")


def byte_lines(proc: subprocess.Popen, deadline: float, events: list[dict]):
    """비차단 바이트 읽기와 줄 큐. 텍스트 readline/select를 사용하지 않는다."""
    fd = proc.stdout.fileno()
    os.set_blocking(fd, False)
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ)
    buffer = bytearray()
    try:
        while time.monotonic() < deadline:
            if proc.poll() is not None and not buffer:
                break
            ready = selector.select(max(0.0, min(0.25, deadline - time.monotonic())))
            if not ready:
                continue
            while True:
                try:
                    chunk = os.read(fd, 65536)
                except BlockingIOError:
                    break
                if not chunk:
                    break
                received = time.time_ns()
                buffer.extend(chunk)
                while b"\n" in buffer:
                    line, _, rest = buffer.partition(b"\n")
                    buffer = bytearray(rest)
                    text = line.decode("utf-8", errors="replace")
                    events.append({"direction": "in", "rawByteReceivedAtNs": received, "line": redact(text)})
                    yield text
                if len(chunk) < 65536:
                    break
            if proc.poll() is not None and not buffer:
                break
        if buffer:
            text = buffer.decode("utf-8", errors="replace")
            events.append({"direction": "in", "rawByteReceivedAtNs": time.time_ns(), "line": redact(text)})
            yield text
    finally:
        selector.close()


class AppServer:
    def __init__(self, env: dict | None = None, enable: list[str] | None = None):
        command = ["codex", "app-server", "--listen", "stdio://"]
        for feature in enable or []:
            command += ["--enable", feature]
        self.proc = subprocess.Popen(command, cwd=WORKTREE, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     start_new_session=True)
        self.events: list[dict] = []
        self.next_id = 1

    def send(self, method: str, params=None, request_id=None):
        value = {"jsonrpc": "2.0", "method": method}
        if request_id is not None:
            value["id"] = request_id
        if params is not None:
            value["params"] = params
        line = json_line(value)
        self.proc.stdin.write((line + "\n").encode())
        self.proc.stdin.flush()
        self.events.append({"direction": "out", "tsNs": time.time_ns(), "line": line})
        if request_id is not None:
            self.next_id = max(self.next_id, int(request_id) + 1)
        return request_id

    def request(self, method: str, params: dict, timeout: float = TIMEOUT_S, handler=None):
        request_id = self.next_id
        self.next_id += 1
        self.send(method, params, request_id)
        for text in byte_lines(self.proc, time.monotonic() + timeout, self.events):
            try:
                message = json.loads(text)
            except json.JSONDecodeError:
                continue
            if message.get("method") and handler:
                handler(self, message)
            elif message.get("method") and message.get("id") is not None:
                self.send_error(message["id"], -32601, "experiment driver did not handle request")
            if message.get("id") == request_id:
                return message
        return None

    def send_result(self, request_id, result):
        self.send_result_or_error(request_id, result=result)

    def send_error(self, request_id, code, message):
        self.send_result_or_error(request_id, error={"code": code, "message": message})

    def send_result_or_error(self, request_id, result=None, error=None):
        value = {"jsonrpc": "2.0", "id": request_id}
        value["result" if error is None else "error"] = result if error is None else error
        line = json_line(value)
        self.proc.stdin.write((line + "\n").encode())
        self.proc.stdin.flush()
        self.events.append({"direction": "out", "tsNs": time.time_ns(), "line": line})

    def initialize(self):
        result = self.request("initialize", {"clientInfo": {"name": "experiment-252", "title": "Experiment 252", "version": "1"}, "capabilities": {"experimentalApi": True}})
        self.send("initialized")
        return result

    def drain(self, timeout: float = TIMEOUT_S, handler=None, stop=None):
        messages = []
        for text in byte_lines(self.proc, time.monotonic() + timeout, self.events):
            try:
                message = json.loads(text)
            except json.JSONDecodeError:
                continue
            messages.append(message)
            if message.get("method") and message.get("id") is not None:
                if handler:
                    handler(self, message)
                else:
                    self.send_error(message["id"], -32601, "experiment driver did not handle request")
            if stop and stop(message):
                break
        return messages

    def close(self):
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
                self.proc.wait(timeout=2)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        stderr = self.proc.stderr.read().decode("utf-8", errors="replace") if self.proc.stderr else ""
        return redact(stderr)


def thread_params(cwd: Path, *, mcp: dict | None = None, feature: bool = False) -> dict:
    config = {"mcp_servers": mcp} if mcp else None
    return {"model": CODEX_MODEL, "cwd": str(cwd), "approvalPolicy": "on-request",
            "approvalsReviewer": "user", "sandbox": "workspace-write", "ephemeral": True,
            "experimentalRawEvents": True, "config": config}


def setup_codex_home(name: str, config: str = "") -> tuple[Path, dict]:
    home = WORKTREE / ".runtime" / name
    home.mkdir(parents=True, exist_ok=True)
    auth = home / "auth.json"
    source = Path.home() / ".codex" / "auth.json"
    if not auth.exists():
        auth.symlink_to(source)
    if config:
        (home / "config.toml").write_text(config, encoding="utf-8")
    env = os.environ.copy()
    env["CODEX_HOME"] = str(home)
    return home, env


def safe_command_request(server: AppServer, message: dict):
    method = message.get("method", "")
    if method.endswith("requestApproval"):
        server.send_result(message["id"], {"decision": "accept"})
    elif method == "item/tool/requestUserInput":
        questions = message.get("params", {}).get("questions", [])
        server.send_result(message["id"], {"answers": {q.get("id", str(i)): "experiment-answer" for i, q in enumerate(questions)}})
    else:
        server.send_error(message["id"], -32601, "unhandled experiment request")


def event_lines(events: list[dict]) -> list[dict]:
    result = []
    for event in events:
        if event.get("direction") != "in":
            continue
        try:
            result.append(json.loads(event["line"]))
        except json.JSONDecodeError:
            pass
    return result
