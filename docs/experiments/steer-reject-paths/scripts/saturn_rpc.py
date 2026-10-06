"""engine 소켓에 직접 붙는 JSON-RPC 클라이언트와 engine 시작·종료. 키는 환경 변수로만 넘기고 출력하지 않는다."""

from __future__ import annotations

import json
import os
import signal
import socket
import subprocess
import time
from pathlib import Path

KEY_SERVICE = "saturn-verify-router"


def router_key() -> str:
    out = subprocess.run(
        ["security", "find-generic-password", "-s", KEY_SERVICE, "-w"],
        capture_output=True,
        text=True,
        check=True,
    )
    return out.stdout.strip()


class Engine:
    """이 드라이버가 띄운 `saturn-engine` 하나. 종료는 자기 PID만."""

    def __init__(self, binary: Path, home: Path, key: str):
        self.home = home
        home.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ)
        env["SATURN_HOME"] = str(home)
        env["SATURN_KEY"] = key
        env["RUST_LOG"] = "warn,saturn_engine=debug"
        self.log = open(home / "engine.stdout", "w")
        self.proc = subprocess.Popen(
            [str(binary), "--home", str(home)],
            env=env,
            stdout=self.log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.sock = home / "engine.sock"
        deadline = time.time() + 40
        while time.time() < deadline and not self.sock.exists():
            if self.proc.poll() is not None:
                raise RuntimeError("engine exited before opening the socket")
            time.sleep(0.2)
        if not self.sock.exists():
            raise RuntimeError("engine socket did not appear")

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(30)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(10)
        self.log.close()


class Client:
    def __init__(self, sock: Path):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(str(sock))
        self.s.settimeout(0.5)
        self.buf = b""
        self.next_id = 0
        self.log: list[dict] = []

    def send(self, method: str, params: dict | None = None) -> int:
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            msg["params"] = params
        self.s.sendall((json.dumps(msg) + "\n").encode())
        self.log.append({"t": time.time(), "dir": "out", "msg": msg})
        return self.next_id

    def poll(self) -> list[dict]:
        """읽을 수 있는 줄을 모두 읽어 돌려주고 기록에 남긴다."""
        out = []
        try:
            data = self.s.recv(1 << 20)
            if data:
                self.buf += data
        except socket.timeout:
            pass
        while b"\n" in self.buf:
            line, self.buf = self.buf.split(b"\n", 1)
            if line.strip():
                msg = json.loads(line)
                self.log.append({"t": time.time(), "dir": "in", "msg": msg})
                out.append(msg)
        return out

    def close(self) -> None:
        self.s.close()


ATTACH_ENV_NAMES = ["PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR"]


def attach_env() -> list[list[str]]:
    return [[n, os.environ[n]] for n in ATTACH_ENV_NAMES if n in os.environ]
