"""끼워 넣기 거절 경로 수집. `collect.py <pilot|formal> [조건...]`.

실제 `saturn-engine`(Codex, gpt-5.6-luna)을 소켓으로 직접 부른다. router는 부르지 않는다(`skip_relation`과 `SendNow`).
결과는 `data/raw/<run>/<조건><번호>.json`에 남기고, 작업 폴더와 홈은 worktree의 `.runtime/s5/`에 둔다.
"""

from __future__ import annotations

import gzip
import json
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

import saturn_rpc as rpc

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
REPO = EXP.parents[2]
BIN = REPO / "target" / "release" / "saturn-engine"
RUNTIME = REPO / ".runtime" / "s5"
OVERRIDES = [["permission.mode", "full"], ["model.mode", "manual"], ["model.default", "codex/gpt-5.6-luna"]]

LONG_TEXT = "Write the numbers 1 to 60 as your final answer, one per line, each followed by a different short English word. Do not use any tools."
QUICK_TEXT = "Reply with the single word ok. Do not use any tools."
HISTORY_TEXT = "Write the numbers 1 to 25 as your final answer, one per line, each followed by a different short English word. Do not use any tools."
MARK_TEXT = "Append the single line {mark} to notes.txt with one shell command, then reply with the single word done."

# 조건: 이름 -> (반복 수, 경합 오프셋 초 목록)
RACE_OFFSETS = [round(1.0 + 0.5 * i, 1) for i in range(12)]
TASKS = {"steer_ok": 3, "steer_review": 3, "steer_compact": 3, "steer_compact_late": 3, "steer_race": len(RACE_OFFSETS), "steer_compact_burst": 12}
TAG = {"steer_ok": "ok", "steer_review": "rv", "steer_compact": "cf", "steer_compact_late": "cl", "steer_race": "rc", "steer_compact_burst": "cb"}
COMPACT_DELAY = {"steer_compact": 0.0, "steer_compact_late": 1.0, "steer_compact_burst": 0.0}
CALL_CAP = 60
TERMINAL_INPUT = {"Applied", "Rejected", "Cancelled", "Held"}


def git(path: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(path), *args], check=True, capture_output=True)


def make_practice(path: Path) -> None:
    path.mkdir(parents=True)
    git(path, "init", "-q")
    git(path, "config", "user.name", "practice")
    git(path, "config", "user.email", "practice@example.invalid")
    (path / "notes.txt").write_text("start\n")
    (path / "app.py").write_text("def add(a, b):\n    return a + b\n")
    git(path, "add", "-A")
    git(path, "commit", "-q", "-m", "init")
    # 검토 대상이 될 미커밋 변경
    (path / "app.py").write_text("def add(a, b):\n    return a - b\n\n\ndef mul(a, b):\n    return a * b\n")


class Session:
    def __init__(self, base: Path, key: str):
        self.base = base
        make_practice(base / "practice")
        self.engine = rpc.Engine(BIN, base / "h", key)
        self.client = rpc.Client(self.engine.sock)
        self.inputs: dict[int, dict] = {}
        self.tasks: dict[int, str] = {}
        self.refs: dict[int, int] = {}
        self.t0 = time.time()
        self.calls = 0
        self.client.send(
            "Attach",
            {"chat": None, "workdir": str(base / "practice"), "env": rpc.attach_env(), "overrides": OVERRIDES},
        )
        self.pump(3.0)
        self.next_ref = 1
        self.chat = 1

    def pump(self, seconds: float = 0.0, until=None) -> bool:
        end = time.time() + seconds
        while True:
            for msg in self.client.poll():
                self.apply(msg)
            if until is not None and until():
                return True
            if time.time() >= end:
                return until is None
            time.sleep(0.05)

    def apply(self, msg: dict) -> None:
        method = msg.get("method")
        params = msg.get("params") or {}
        if method == "InputAccepted":
            self.refs[params["client_ref"]] = params["input"]
        elif method == "InputChanged":
            rec = self.inputs.setdefault(params["input"], {"states": []})
            rec["text"] = params["text"]
            rec["task"] = params.get("task")
            entry = (params["state"], params.get("disposition"), params.get("reason"))
            if not rec["states"] or rec["states"][-1][0] != entry:
                rec["states"].append([entry, round(time.time() - self.t0, 2)])
            rec["state"] = params["state"]
        elif method == "TaskChanged":
            self.tasks[params["task"]] = params["state"]

    def submit(self, text: str) -> int:
        if self.calls >= CALL_CAP:
            raise RuntimeError("provider request cap reached")
        self.calls += 1
        ref = self.next_ref
        self.next_ref += 1
        self.client.send("SubmitInput", {"chat": self.chat, "client_ref": ref, "text": text, "skip_relation": True})
        self.pump(10, until=lambda: ref in self.refs)
        return self.refs[ref]

    def send_now(self, input_id: int) -> float:
        at = round(time.time() - self.t0, 2)
        self.client.send("SendNow", {"input": input_id})
        return at

    def running(self) -> bool:
        return any(state == "Running" for state in self.tasks.values())

    def wait_state(self, input_id: int, states: set[str], timeout: float = 120) -> bool:
        return self.pump(timeout, until=lambda: self.inputs.get(input_id, {}).get("state") in states)

    def settle(self, timeout: float = 300) -> bool:
        """미종결 입력이 없고 실행 중 작업이 없는 상태가 8초 이어질 때."""
        quiet_since = None
        end = time.time() + timeout
        while time.time() < end:
            self.pump(1.0)
            busy = self.running() or any(rec.get("state") not in TERMINAL_INPUT for rec in self.inputs.values())
            if busy:
                quiet_since = None
            else:
                quiet_since = quiet_since or time.time()
                if time.time() - quiet_since >= 8:
                    return True
        return False

    def finish(self) -> dict:
        self.pump(2.0)
        db = self.base / "h" / "saturn.db"
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        con.row_factory = sqlite3.Row
        tables = {
            "inputs": [dict(r) for r in con.execute("SELECT * FROM inputs ORDER BY id")],
            "runs": [dict(r) for r in con.execute("SELECT id,input_id,task_id,agent_id,session_id,provider,started_at,ended_at,end_kind FROM runs ORDER BY id")],
        }
        con.close()
        notes = (self.base / "practice" / "notes.txt").read_text()
        logs = ""
        for path in sorted((self.base / "h" / "logs").glob("engine-*.log")):
            logs += path.read_text(errors="replace")
        steer_lines = [line for line in logs.splitlines() if "steer" in line.lower()]
        self.engine.stop()
        out = {
            "inputs_seen": {str(k): v for k, v in self.inputs.items()},
            "tasks": {str(k): v for k, v in self.tasks.items()},
            "db": tables,
            "notes": notes,
            "engine_steer_log": steer_lines,
            "messages": self.client.log,
            "provider_calls": self.calls,
        }
        self.client.close()
        return out


def key_leaks(base: Path, key: str) -> int:
    n = 0
    for path in base.rglob("*"):
        if path.is_file() and path.suffix not in {".db", ".sqlite"} and path.stat().st_size < 50_000_000:
            try:
                if key.encode() in path.read_bytes():
                    n += 1
            except OSError:
                pass
    return n


def run_one(run: str, cond: str, rep: int, key: str) -> dict:
    base = RUNTIME / run / f"{TAG[cond]}{rep}"
    if base.exists():
        raise RuntimeError(f"already collected: {base}")
    s = Session(base, key)
    plan: dict = {"cond": cond, "rep": rep}
    try:
        if cond == "steer_ok":
            a = s.submit(LONG_TEXT)
            plan["text_wait"] = s.pump(120, until=lambda: s.running())
            s.pump(4.0)  # 첫 글 조각이 오고 몇 초 뒤
            b = s.submit(MARK_TEXT.format(mark=f"MARK-B-{rep}"))
            s.wait_state(b, {"Queued"}, 10)
            plan["send_now_at"] = s.send_now(b)
        elif cond == "steer_review":
            a = s.submit("/review")
            s.pump(120, until=lambda: s.running())
            s.pump(4.0)
            b = s.submit(MARK_TEXT.format(mark=f"MARK-B-{rep}"))
            s.wait_state(b, {"Queued"}, 10)
            plan["send_now_at"] = s.send_now(b)
        elif cond in COMPACT_DELAY:
            h = s.submit(HISTORY_TEXT)
            s.settle(200)
            a = s.submit("/compact")
            s.wait_state(a, {"Applied"}, 60)
            s.pump(COMPACT_DELAY[cond])
            b = s.submit(MARK_TEXT.format(mark=f"MARK-B-{rep}"))
            s.wait_state(b, {"Queued"}, 10)
            plan["send_now_at"] = s.send_now(b)
        elif cond == "steer_race":
            h = s.submit(QUICK_TEXT)
            s.settle(200)
            began = time.time()
            a = s.submit(QUICK_TEXT)
            offset = RACE_OFFSETS[rep - 1]
            plan["offset"] = offset
            s.pump(max(0.0, began + offset - time.time()))
            b = s.submit(MARK_TEXT.format(mark=f"MARK-B-{rep}"))
            s.wait_state(b, {"Queued", "Delivering", "Applied"}, 10)
            plan["send_now_at"] = s.send_now(b)
        else:
            raise ValueError(cond)
        plan["a"], plan["b"] = a, b
        plan["settled"] = s.settle(300)
    finally:
        pass
    result = s.finish()
    result["plan"] = plan
    result["key_files_with_key"] = key_leaks(base, key)
    return result


def main() -> None:
    run = sys.argv[1]
    wanted = sys.argv[2:] or list(TASKS)
    out_dir = EXP / "data" / "raw" / run
    out_dir.mkdir(parents=True, exist_ok=True)
    key = rpc.router_key()
    for cond in wanted:
        for rep in range(1, (1 if run.startswith('pilot') else TASKS[cond]) + 1):
            target = out_dir / f"{cond}-{rep}.json.gz"
            if target.exists():
                continue
            result = run_one(run, cond, rep, key)
            with gzip.open(target, "wt") as handle:
                json.dump(result, handle, ensure_ascii=False)
            states = {k: [e[0][0] for e in v["states"]] for k, v in result["inputs_seen"].items()}
            print(cond, rep, result["plan"].get("offset"), states, result["notes"].count("MARK-B"), flush=True)


if __name__ == "__main__":
    main()
