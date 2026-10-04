from __future__ import annotations

import datetime as dt
import json
import os
import secrets
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]
RUNTIME = WORKTREE / ".runtime"
RAW = ROOT / "data" / "raw"
SECURITY = "/usr/bin/security"
READER_PYTHON = "/Library/Developer/CommandLineTools/usr/bin/python3"
READER_SCRIPT = ROOT / "scripts" / "keychain_read.py"
PREFIX = "saturn-defense-test-"
ACCOUNT = "saturn-defense-test"
ITEM_NUMBERS = range(1, 10)
DIALOG_WAIT = 12


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def commit7() -> str:
    out = subprocess.run(
        ["git", "-C", str(WORKTREE), "rev-parse", "--short=7", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    return out.stdout.strip()


def service(n: int) -> str:
    return f"{PREFIX}{n}"


def create_item(n: int, trust: list[str] | None) -> bool:
    """trust가 None이면 기본 신뢰 목록, 빈 목록이면 `-T ""`, 아니면 그 실행 파일만. 값은 무작위이고 어디에도 쓰지 않는다."""
    args = [SECURITY, "add-generic-password", "-a", ACCOUNT, "-s", service(n), "-w", secrets.token_hex(16)]
    if trust is not None:
        for app in trust or [""]:
            args += ["-T", app]
    return subprocess.run(args, capture_output=True, timeout=20).returncode == 0


def delete_item(n: int) -> None:
    subprocess.run(
        [SECURITY, "delete-generic-password", "-a", ACCOUNT, "-s", service(n)],
        capture_output=True,
        timeout=20,
    )


def item_exists(n: int) -> bool:
    """속성만 조회한다(값 조회 아님)."""
    return (
        subprocess.run(
            [SECURITY, "find-generic-password", "-a", ACCOUNT, "-s", service(n)],
            capture_output=True,
            timeout=20,
        ).returncode
        == 0
    )


def cleanup_all() -> dict[str, bool]:
    for n in ITEM_NUMBERS:
        if item_exists(n):
            delete_item(n)
    return {service(n): item_exists(n) for n in ITEM_NUMBERS}


def run_timed(args: list[str], timeout: float, env: dict | None = None, cwd: Path | None = None, capture: bool = False):
    """자기가 띄운 프로세스만 종료한다. 반환: (종료 코드 또는 None, 시간 초과 여부, 경과 초, stdout)."""
    start = time.monotonic()
    proc = subprocess.Popen(
        args,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        env=env,
        cwd=cwd,
    )
    try:
        out, _ = proc.communicate(timeout=timeout)
        return proc.returncode, False, time.monotonic() - start, (out or b"").decode(errors="replace")
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        out, _ = proc.communicate()
        return None, True, time.monotonic() - start, (out or b"").decode(errors="replace")


def classify(code, timed_out: bool) -> str:
    if timed_out:
        return "dialog"
    return "accessed" if code == 0 else "denied"


class Writer:
    def __init__(self, layer: str, run_id: str):
        RAW.mkdir(parents=True, exist_ok=True)
        self.layer = layer
        self.run_id = run_id
        self.path = RAW / f"{layer}-{run_id}.jsonl"
        self.n = 0

    def row(self, condition: str, **fields) -> dict:
        self.n += 1
        row = {
            "run_id": self.run_id,
            "trial_id": f"{self.layer}-{self.n:03d}",
            "condition": condition,
            "ts_utc": utc_now(),
            "layer": self.layer,
            **fields,
        }
        with self.path.open("a", encoding="utf-8", newline="\n") as f:
            f.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
        return row
