"""Codex app-server를 직접 구동하는 공용 driver. 호출 횟수는 `.runtime/codex-calls.jsonl`에 누적한다."""
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
RUNTIME = WORKTREE / ".runtime"
CALL_LOG = RUNTIME / "codex-calls.jsonl"
MODEL = "gpt-5.6-luna"
CALL_LIMIT = 120
SECRET_KEYS = {"token", "access_token", "refresh_token", "id_token", "accesstoken", "refreshtoken", "idtoken", "secret", "authorization", "api_key", "apikey", "password", "email"}


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def calls_used() -> int:
    if not CALL_LOG.exists():
        return 0
    return sum(1 for line in CALL_LOG.read_text(encoding="utf-8").splitlines() if line.strip())


def record_call(run_id: str, trial_id: str, kind: str) -> int:
    """모델 호출(`turn/start`, `review/start`, `thread/compact/start`) 직전에 부른다. 상한이면 중단한다."""
    used = calls_used()
    if used >= CALL_LIMIT:
        raise RuntimeError(f"Codex 호출 상한 {CALL_LIMIT}회에 도달함")
    RUNTIME.mkdir(parents=True, exist_ok=True)
    with CALL_LOG.open("a", encoding="utf-8") as out:
        out.write(json.dumps({"ordinal": used + 1, "run_id": run_id, "trial_id": trial_id, "kind": kind, "ts_utc": utc_now()}) + "\n")
    return used + 1


def redact(value):
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if str(key).lower() in SECRET_KEYS:
                out[key] = "[redacted]"
            else:
                out[key] = redact(item)
        return out
    if isinstance(value, list):
        return [redact(item) for item in value]
    if isinstance(value, str):
        return value.replace(str(Path.home()), "~")
    return value


def jline(value) -> str:
    return json.dumps(redact(value), ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def make_home(name: str, config: str = "", rules: str = "", hooks: dict | None = None, link_auth: bool = True) -> Path:
    """전용 CODEX_HOME. auth.json은 심볼릭 링크만 만들고 원본은 읽지 않는다."""
    home = RUNTIME / "codex-homes" / name
    (home / "rules").mkdir(parents=True, exist_ok=True)
    base = 'approvals_reviewer = "user"\nmcp_optional_startup_grace_ms = 12000\n'
    (home / "config.toml").write_text(base + config, encoding="utf-8")
    (home / "rules" / "default.rules").write_text(rules, encoding="utf-8")
    if hooks is not None:
        (home / "hooks.json").write_text(json.dumps(hooks), encoding="utf-8")
    os.chmod(home, 0o700)
    source = Path.home() / ".codex" / "auth.json"
    auth = home / "auth.json"
    if link_auth:
        if not auth.is_symlink():
            auth.symlink_to(source)
    return home


class AppServer:
    def __init__(self, home: Path, cwd: Path):
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        env.pop("HERDR_ENV", None)
        self.proc = subprocess.Popen(
            ["codex", "app-server"], cwd=cwd, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
        )
        self.events: list[dict] = []
        self.next_id = 1
        self.buffer = bytearray()
        self.sel = selectors.DefaultSelector()
        os.set_blocking(self.proc.stdout.fileno(), False)
        self.sel.register(self.proc.stdout.fileno(), selectors.EVENT_READ)
        self.pending: list[dict] = []
        self.responses: dict = {}

    @property
    def pid(self) -> int:
        return self.proc.pid

    def send(self, message: dict) -> None:
        self.proc.stdin.write((json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n").encode())
        self.proc.stdin.flush()
        self.events.append({"dir": "out", "ns": time.time_ns(), "msg": message})

    def _read(self, timeout: float) -> list[dict]:
        out = []
        if not self.sel.select(timeout):
            return out
        while True:
            try:
                chunk = os.read(self.proc.stdout.fileno(), 65536)
            except BlockingIOError:
                break
            if not chunk:
                break
            self.buffer.extend(chunk)
            if len(chunk) < 65536:
                break
        while b"\n" in self.buffer:
            line, _, rest = self.buffer.partition(b"\n")
            self.buffer = bytearray(rest)
            try:
                msg = json.loads(line.decode("utf-8", errors="replace"))
            except json.JSONDecodeError:
                msg = {"nonJson": line.decode("utf-8", errors="replace")}
            self.events.append({"dir": "in", "ns": time.time_ns(), "msg": msg})
            if "id" in msg and "method" not in msg:
                self.responses[msg["id"]] = msg
            out.append(msg)
        return out

    def messages(self, deadline: float, ticks: bool = False):
        """deadline(monotonic)까지 들어오는 메시지를 차례로 낸다. ticks이면 메시지가 없는 틈마다 {"tick": True}도 낸다."""
        while True:
            while self.pending:
                yield self.pending.pop(0)
            if time.monotonic() >= deadline:
                return
            got = self._read(max(0.0, min(0.2, deadline - time.monotonic())))
            self.pending.extend(got)
            if not got and ticks:
                yield {"tick": True}
            if self.proc.poll() is not None and not self.pending and not self.buffer:
                return

    def respond(self, request: dict, result=None, error=None) -> None:
        if error is not None:
            self.send({"jsonrpc": "2.0", "id": request["id"], "error": error})
        else:
            self.send({"jsonrpc": "2.0", "id": request["id"], "result": result})

    def send_async(self, method: str, params: dict) -> int:
        """응답을 기다리지 않고 보낸다. 응답은 `responses[id]`와 메시지 흐름에 나타난다."""
        rid = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return rid

    def request(self, method: str, params: dict, timeout: float = 60.0, on_request=None) -> dict | None:
        rid = self.send_async(method, params)
        for msg in self.messages(time.monotonic() + timeout):
            if msg.get("method") and "id" in msg:
                (on_request or self.default_request)(msg)
            if rid in self.responses:
                return self.responses[rid]
        return self.responses.get(rid)

    def default_request(self, msg: dict) -> None:
        self.respond(msg, error={"code": -32601, "message": "driver does not handle this request"})

    def initialize(self) -> dict | None:
        res = self.request("initialize", {"clientInfo": {"name": "saturn-codex-behavior", "title": "Saturn Codex behavior experiment", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "initialized"})
        return res

    def drain(self, seconds: float, on_request=None) -> list[dict]:
        got = []
        for msg in self.messages(time.monotonic() + seconds):
            got.append(msg)
            if msg.get("method") and "id" in msg:
                (on_request or self.default_request)(msg)
        return got

    def stderr_text(self) -> str:
        fd = self.proc.stderr.fileno()
        os.set_blocking(fd, False)
        chunks = []
        while True:
            try:
                chunk = os.read(fd, 65536)
            except BlockingIOError:
                break
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks).decode("utf-8", errors="replace")

    def close(self) -> str:
        """이 driver가 시작한 app-server의 프로세스 묶음만 종료한다."""
        text = ""
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.proc.wait(timeout=5)
        text = self.stderr_text()
        self.sel.close()
        return text


def observe(server: AppServer, seconds: float, on_request, on_message=None) -> None:
    """요청에 답하며 seconds 동안 메시지를 읽는다. on_message가 True를 돌려주면 일찍 끝낸다."""
    for msg in server.messages(time.monotonic() + seconds, ticks=True):
        if msg.get("method") and "id" in msg:
            on_request(msg)
            continue
        if on_message is not None and on_message(msg):
            return


def methods(events: list[dict], direction: str = "in") -> list[str]:
    return [e["msg"].get("method") for e in events if e["dir"] == direction and e["msg"].get("method")]


def turn_events(server: AppServer, thread_id: str, prompt: str, on_request, timeout: float = 150.0, on_message=None, params_extra=None, method: str = "turn/start", params_override: dict | None = None):
    """`turn/start`(또는 `review/start` 등)를 보내고 해당 thread의 `turn/completed`까지 읽는다.
    on_message(msg)는 요청이 아닌 모든 메시지마다 불리며 True를 돌려주면 읽기를 멈춘다. (turn id, 상태, 응답)을 돌려준다."""
    params = params_override if params_override is not None else {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}
    params = {**params, **(params_extra or {})}
    rid = server.send_async(method, params)
    status = "timeout"
    for msg in server.messages(time.monotonic() + timeout, ticks=True):
        if msg.get("method") and "id" in msg:
            on_request(msg)
            continue
        if msg.get("id") == rid and "method" not in msg and "error" in msg:
            status = "rejected"
            break
        if on_message is not None and on_message(msg):
            status = "stopped"
            break
        if msg.get("method") == "turn/completed" and msg["params"].get("threadId") == thread_id:
            status = "completed"
            break
    response = server.responses.get(rid)
    turn_id = ((response or {}).get("result") or {}).get("turn", {}).get("id")
    return turn_id, status, response
