"""실제 `saturn --plain`과 engine을 입력 한 줄씩 부른다. 재전송하지 않는다.

입력마다 `saturn` 클라이언트를 새로 띄우고(두 번째부터 `--continue`), 그 입력의 작업이 끝나면 클라이언트가 끝난다.
engine은 `SATURN_HOME`마다 하나씩 뜨고, 끝낼 때 이 드라이버가 띄운 PID만 종료한다.
"""

from __future__ import annotations

import os
import re
import signal
import subprocess
import threading
import time
from pathlib import Path

KEY_SERVICE = "saturn-verify-router"
PERMISSION_LINE = re.compile(r" · (Reason|사유):")


def router_key() -> str:
    """키체인에서 읽어 환경 변수로만 넘긴다. 출력과 파일에 쓰지 않는다."""
    out = subprocess.run(
        ["security", "find-generic-password", "-s", KEY_SERVICE, "-w"],
        capture_output=True,
        text=True,
        check=True,
    )
    return out.stdout.strip()


class Chat:
    def __init__(self, binary_dir: Path, home: Path, workdir: Path, key: str):
        self.home = home
        self.workdir = workdir
        self.binary_dir = binary_dir
        self.env = dict(os.environ)
        self.env["PATH"] = f"{binary_dir}:{self.env['PATH']}"
        self.env["SATURN_HOME"] = str(home)
        self.env["SATURN_KEY"] = key
        home.mkdir(parents=True, exist_ok=True)
        self.started = False

    def send(self, text: str, overrides: list[str], timeout: float = 900) -> dict:
        """입력 하나. 클라이언트 PID만 종료 대상이고 시간이 넘으면 `timeout`으로 남긴다."""
        cmd = ["saturn", "--plain"]
        if self.started:
            cmd.append("--continue")
        for item in overrides:
            cmd += ["-c", item]
        began = time.time()
        proc = subprocess.Popen(
            cmd,
            cwd=self.workdir,
            env=self.env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        self.started = True
        lines: list[str] = []
        reader = threading.Thread(target=lambda: lines.extend(iter(proc.stdout.readline, "")), daemon=True)
        reader.start()
        proc.stdin.write(text + "\n")
        proc.stdin.close()
        status = "ok"
        asked_at = None
        while proc.poll() is None:
            time.sleep(1)
            now = time.time()
            if now - began > timeout:
                status = "timeout"
                break
            # plain 출력은 허가 요청에 답할 수 없다. 요청 줄이 보이고 5초 안에 끝나지 않으면 그 입력은 멈춘 것으로 본다
            if asked_at is None and any(PERMISSION_LINE.search(line) for line in list(lines)):
                asked_at = now
            if asked_at is not None and now - asked_at > 5:
                status = "permission_wait"
                break
        if status != "ok":
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
        reader.join(5)
        ended = time.time()
        if status == "ok" and proc.returncode != 0:
            status = "failed"
        return {
            "text": text,
            "overrides": overrides,
            "started_unix": began,
            "ended_unix": ended,
            "exit_code": proc.returncode,
            "status": status,
            "stdout": "".join(lines),
        }

    def engine_pids(self) -> list[int]:
        """이 홈으로 띄운 engine의 PID. 명령줄이 정확히 같은 것만 센다."""
        wanted = f"--home {self.home}"
        pids = []
        listing = subprocess.run(["ps", "-eo", "pid,command"], capture_output=True, text=True).stdout
        for line in listing.splitlines()[1:]:
            pid, _, command = line.strip().partition(" ")
            if command.strip().endswith(f"saturn-engine {wanted}"):
                pids.append(int(pid))
        return pids

    def stop_engine(self) -> list[int]:
        pids = self.engine_pids()
        for pid in pids:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.time() + 30
        while time.time() < deadline and self.engine_pids():
            time.sleep(0.5)
        return pids


class TuiSwitch:
    """전체 화면 TUI를 자기 전용 tmux 소켓에서 띄워 `/model`로 모델을 고르고 입력을 보낸다.

    plain 출력에는 모델 고르기 창이 없어서 provider 전환만 이 경로를 쓴다. 완료는 기록 저장소를 읽기 전용으로 보고 판정한다.
    """

    def __init__(self, chat: Chat, socket_name: str, overrides: list[str]):
        self.chat = chat
        self.sock = socket_name
        self.overrides = overrides
        self.session = "sw"
        self.denied = 0

    def _tmux(self, *args: str) -> str:
        return subprocess.run(["tmux", "-L", self.sock, *args], capture_output=True, text=True).stdout

    def start(self) -> None:
        env = f'export SATURN_HOME="{self.chat.home}" PATH="{self.chat.binary_dir}:$PATH"; '
        key = f'export SATURN_KEY="$(security find-generic-password -s {KEY_SERVICE} -w)"; '
        flags = " ".join(f"-c {o}" for o in self.overrides)
        self._tmux("new-session", "-d", "-s", self.session, "-x", "170", "-y", "50", "-c", str(self.chat.workdir), "zsh -f")
        self._tmux("send-keys", "-t", self.session, f"{env}{key}clear; saturn --continue {flags}", "Enter")
        time.sleep(8)

    def screen(self) -> str:
        return self._tmux("capture-pane", "-t", self.session, "-p")

    def pick_model(self, provider: str, label: str) -> str:
        """`/model <provider>`로 창을 열고 목록이 뜨면 `label`이 든 줄까지 내려 고른다. 화면 기록을 돌려준다."""
        self._tmux("send-keys", "-t", self.session, "-l", f"/model {provider}")
        time.sleep(1)
        self._tmux("send-keys", "-t", self.session, "Enter")  # 첫 Enter는 명령 완성
        time.sleep(1)
        self._tmux("send-keys", "-t", self.session, "Enter")
        rows: list[str] = []
        opened = ""
        for _ in range(30):
            time.sleep(1)
            opened = self.screen()
            rows = [ln for ln in opened.splitlines() if f"{provider} ·" in ln and "Default model" not in ln and "│" in ln]
            if rows:
                break
        down = next((i for i, ln in enumerate(rows) if label.lower() in ln.lower()), None)
        if down is None:
            return opened + "\n----\nlist did not show " + label
        for _ in range(down):
            self._tmux("send-keys", "-t", self.session, "Down")
            time.sleep(0.3)
        self._tmux("send-keys", "-t", self.session, "Enter")
        time.sleep(2)
        return opened + "\n----\n" + self.screen()

    def _state(self) -> tuple[int, str | None, int]:
        import sqlite3

        con = sqlite3.connect(f"file:{self.chat.home / 'saturn.db'}?mode=ro", uri=True, timeout=5)
        try:
            n, = con.execute("SELECT count(*) FROM inputs").fetchone()
            last = con.execute("SELECT state FROM inputs ORDER BY id DESC LIMIT 1").fetchone()
            open_runs, = con.execute("SELECT count(*) FROM runs WHERE ended_at IS NULL").fetchone()
        finally:
            con.close()
        return n, (last[0] if last else None), open_runs

    def send(self, text: str, timeout: float = 900) -> dict:
        before, _, _ = self._state()
        began = time.time()
        self._tmux("send-keys", "-t", self.session, "C-u")  # 거부 뒤 열리는 `[A]에게: ` 접두 초안을 지운다
        self._tmux("send-keys", "-t", self.session, "-l", text)
        time.sleep(0.5)
        self._tmux("send-keys", "-t", self.session, "Enter")
        status = "timeout"
        stable = 0
        while time.time() - began < timeout:
            time.sleep(2)
            if "Waiting for permission" in self.screen() or "허가 대기" in self.screen():
                self.denied += 1
                self._tmux("send-keys", "-t", self.session, "d")  # 거부. 같은 요청에 두 번 답하지 않도록 잠시 기다린다
                time.sleep(2)
            n, last, open_runs = self._state()
            if n > before and last in ("Applied", "Rejected", "Cancelled") and open_runs == 0:
                stable += 1
                if stable >= 3:
                    status = "ok" if last == "Applied" else last.lower()
                    break
            else:
                stable = 0
        return {"text": text, "started_unix": began, "ended_unix": time.time(), "status": status, "exit_code": None, "stdout": self.screen(), "overrides": self.overrides, "permission_denied": self.denied}

    def close(self) -> None:
        self._tmux("kill-server")
