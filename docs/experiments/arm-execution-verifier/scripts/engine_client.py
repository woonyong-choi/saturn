"""engine JSON-RPC 소켓에 붙는 최소 클라이언트. 줄 단위 JSON-RPC 2.0. 입력 전송과 알림 수집만 한다."""

from __future__ import annotations

import json
import os
import select
import socket
import subprocess
import time
from pathlib import Path


# 실제 TUI가 Attach로 넘기는 변수 이름(protocol의 ATTACH_ENV_NAMES). router 키는 넘기지 않는다
ATTACH_ENV = ["PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "SSH_AUTH_SOCK", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "ALL_PROXY"]


class Engine:
    def __init__(self, binary: Path, home: Path, log: Path, env: dict | None = None):
        self.home = home
        home.mkdir(parents=True, exist_ok=True)
        self.log = open(log, "wb")
        # 자기 프로세스 그룹으로 띄우고 자기 PID만 종료한다
        self.proc = subprocess.Popen(
            [str(binary), "--home", str(home)],
            stdin=subprocess.DEVNULL,
            stdout=self.log,
            stderr=self.log,
            start_new_session=True,
            env=env,
        )
        self.sock_path = home / "engine.sock"
        deadline = time.time() + 30
        while not self.sock_path.exists():
            if time.time() > deadline or self.proc.poll() is not None:
                raise RuntimeError("engine did not start")
            time.sleep(0.2)

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.log.close()


class Client:
    def __init__(self, sock_path: Path):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(str(sock_path))
        self.buf = b""
        self.decisions: list[dict] = []
        self.next_id = 0
        self.notes: list[dict] = []

    def send(self, method: str, params: dict | None = None) -> int:
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            msg["params"] = params
        self.s.sendall((json.dumps(msg) + "\n").encode())
        return self.next_id

    def read(self, timeout: float) -> dict | None:
        end = time.time() + timeout
        while b"\n" not in self.buf:
            left = end - time.time()
            if left <= 0 or not select.select([self.s], [], [], left)[0]:
                return None
            chunk = self.s.recv(1 << 20)
            if not chunk:
                raise RuntimeError("engine closed")
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        msg = json.loads(line)
        self.notes.append(msg)
        return msg

    def call(self, method: str, params: dict | None = None, timeout: float = 30) -> dict:
        rid = self.send(method, params)
        end = time.time() + timeout
        while time.time() < end:
            msg = self.read(1)
            if msg and "method" not in msg and msg.get("id") == rid:
                return msg
        raise TimeoutError(method)

    def attach(self, workdir: Path, overrides: list[list[str]]) -> int:
        """채팅을 새로 열고 채팅 번호를 돌려준다."""
        env = [[k, os.environ[k]] for k in ATTACH_ENV if k in os.environ]
        self.call("Attach", {"chat": None, "workdir": str(workdir), "env": env, "overrides": overrides, "add_dirs": []})
        for _ in range(20):
            for n in self.notes:
                if n.get("method") == "HistoryChunk":
                    return n["params"]["chat"]
            self.read(1)
        raise RuntimeError("no chat id")

    def set_model(self, chat: int, provider: str, model: str) -> None:
        self.call("SetModel", {"chat": chat, "model": {"provider": provider, "model": model}})

    def run_input(self, chat: int, text: str, allow=lambda summary: True, timeout: float = 600) -> dict:
        """입력 하나를 보내고 작업이 끝날 때까지 기다린다. 허가 요청은 allow(요약)가 참일 때만 이번만 허용하고 나머지는 거절한다. 재전송하지 않는다."""
        mark = len(self.notes)
        self.send("SubmitInput", {"chat": chat, "client_ref": self.next_id, "text": text, "skip_relation": False})
        tasks: dict[int, str] = {}
        input_state = None
        end = time.time() + timeout
        while time.time() < end:
            m = self.read(2)
            if m is None:
                m = None
            elif m.get("error"):
                return {"status": "rejected", "error": m["error"]}
            elif m.get("method") == "PermissionRequested":
                ok = allow(m["params"].get("summary", ""))
                self.decisions.append({"summary": m["params"].get("summary", ""), "allowed": ok})
                self.send("AnswerPermission", {"request_id": m["params"]["request_id"], "answer": "AllowOnce" if ok else {"Deny": {"note": None}}})
            elif m.get("method") == "InputChanged":
                input_state = m["params"]["state"]
            elif m.get("method") == "TaskChanged":
                tasks[m["params"]["task"]] = m["params"]["state"]
            done = tasks and all(v in ("Done", "Failed", "NeedsCheck") for v in tasks.values())
            if done or input_state in ("Rejected", "Cancelled"):
                time.sleep(1)
                while self.read(0.5):
                    pass
                return {"status": "ok" if done else str(input_state), "tasks": tasks, "notes": self.notes[mark:]}
        return {"status": "timeout", "tasks": tasks, "notes": self.notes[mark:]}

    def close(self) -> None:
        self.s.close()
