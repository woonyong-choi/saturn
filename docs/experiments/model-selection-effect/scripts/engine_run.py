"""실제 saturn(engine)에 과제 하나를 한 팔로 보내고 원자료를 모은다.

운영 `~/.saturn`, `~/.claude`, `~/.codex`는 건드리지 않는다. 실행마다 `.runtime/` 안에 전용 SATURN_HOME과 연습 저장소를 만든다.
router 키는 이 프로세스가 키체인에서 읽어 자식 환경에만 넣고 어디에도 쓰거나 출력하지 않는다.
"""

from __future__ import annotations

import json
import os
import shlex
import sqlite3
import subprocess
import time
from pathlib import Path

import tasks

ROOT = Path(__file__).resolve().parents[4]
RUNTIME = ROOT / ".runtime"  # 소켓 경로 길이 제한 때문에 짧은 이름을 쓴다
SATURN_BIN = Path(os.environ.get("SATURN_BIN", Path.home() / "workspace/oss/saturn.wt/exp-542-engine/target/release"))
KEYCHAIN_SERVICE = "saturn-verify-router"
TRIAL_TIMEOUT_S = 600

CLAUDE = ["claude/haiku", "claude/sonnet", "claude/opus"]
CODEX = ["codex/gpt-6-luna", "codex/gpt-6-astra"]

# 팔: (층, 방식, 고정 모델 또는 기본 모델, 후보). fixed는 매뉴얼 모드의 기본 모델, auto는 오토 모드와 대체 기본 모델.
ARMS = {
    "c-haiku": dict(layer="claude", mode="manual", model="claude/haiku", candidates=CLAUDE),
    "c-sonnet": dict(layer="claude", mode="manual", model="claude/sonnet", candidates=CLAUDE),
    "c-opus": dict(layer="claude", mode="manual", model="claude/opus", candidates=CLAUDE),
    "c-auto": dict(layer="claude", mode="auto", model="claude/sonnet", candidates=CLAUDE),
    "x-luna": dict(layer="codex", mode="manual", model="codex/gpt-6-luna", candidates=CODEX),
    "x-astra": dict(layer="codex", mode="manual", model="codex/gpt-6-astra", candidates=CODEX),
    "x-auto": dict(layer="codex", mode="auto", model="codex/gpt-6-astra", candidates=CODEX),
    "b-auto": dict(layer="both", mode="auto", model="claude/sonnet", candidates=CLAUDE + CODEX),
}

TABLES = {
    "judgments": "select * from judgments order by id",
    "inputs": "select * from inputs order by id",
    "model_selections": "select * from model_selections order by id",
    "model_shadows": "select * from model_shadows order by id",
    "runs": "select * from runs order by id",
    "usage": "select * from usage order by id",
    "sessions": "select * from sessions order by id",
    "handoff_packets": "select * from handoff_packets order by id",
}


def router_key() -> str:
    out = subprocess.run(["security", "find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"], capture_output=True, text=True, check=True)
    return out.stdout.strip()


def flat(rows: list[sqlite3.Row]) -> list[dict]:
    result = []
    for row in rows:
        item = {}
        for key in row.keys():
            value = row[key]
            item[key] = value.decode("utf-8", "replace") if isinstance(value, bytes) else value
        result.append(item)
    return result


def dump_db(home: Path) -> dict:
    db = sqlite3.connect(f"file:{home / 'saturn.db'}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    out = {}
    for name, sql in TABLES.items():
        try:
            out[name] = flat(db.execute(sql).fetchall())
        except sqlite3.Error as error:
            out[name] = {"error": str(error)}
    db.close()
    return out


def engine_pids(home: Path) -> list[int]:
    """이 실행의 SATURN_HOME을 `--home` 값으로 가진 saturn-engine만 찾는다."""
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    pids = []
    for line in out.splitlines():
        pid, _, command = line.strip().partition(" ")
        tokens = command.split()
        if tokens and tokens[0].endswith("saturn-engine") and "--home" in tokens:
            if tokens[tokens.index("--home") + 1 : tokens.index("--home") + 2] == [str(home)]:
                pids.append(int(pid))
    return pids


def stop_engine(home: Path) -> int:
    stopped = 0
    for pid in engine_pids(home):
        try:
            os.kill(pid, 15)
            stopped += 1
        except ProcessLookupError:
            pass
    deadline = time.time() + 15
    while engine_pids(home) and time.time() < deadline:
        time.sleep(0.5)
    for pid in engine_pids(home):
        os.kill(pid, 9)
    return stopped


def trial_dirs(tid: int) -> tuple[Path, Path]:
    """SATURN_HOME(`h/NNN`)과 연습 저장소(`p/NNN`). engine 소켓 경로가 짧아야 한다."""
    return RUNTIME / "h" / f"{tid:03d}", RUNTIME / "p" / f"{tid:03d}"


TMUX = ["tmux", "-L", "542"]
WINDOW_TITLE = "Choose a default model"
MODEL_LABELS = {
    "claude/default": "claude · default", "claude/opus": "claude · opus", "claude/sonnet": "claude · sonnet", "claude/haiku": "claude · haiku",
    "codex/gpt-6-astra": "codex · GPT-6-Astra", "codex/gpt-6-luna": "codex · GPT-6-Luna",
}


def tmux(*args: str) -> str:
    return subprocess.run([*TMUX, *args], capture_output=True, text=True).stdout


def pane(session: str) -> str:
    return tmux("capture-pane", "-p", "-t", session)


def wait_for(session: str, text: str, seconds: float) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if text in pane(session):
            return True
        time.sleep(1)
    return False


def window_rows(session: str) -> list[str]:
    lines = (line.strip("│ ").strip() for line in pane(session).splitlines())
    return [line for line in lines if line.startswith(("claude ·", "codex ·"))]


def pick_default(session: str, model: str) -> bool:
    """처음 고르기 창에서 model 줄로 내려가 Enter. 목록이 다 올 때까지 기다리고 줄 순서는 화면에서 읽는다."""
    label = MODEL_LABELS[model]
    deadline = time.time() + 90
    rows = window_rows(session)
    while (label not in rows or not any(r.startswith("codex ·") for r in rows)) and time.time() < deadline:
        time.sleep(1)
        rows = window_rows(session)
    if label not in rows:
        return False
    for _ in range(rows.index(label)):
        tmux("send-keys", "-t", session, "Down")
    tmux("send-keys", "-t", session, "Enter")
    return wait_for(session, "Default model set to", 20)


def db_state(home: Path) -> dict:
    """입력 상태와 실행 끝 시각을 읽기 전용으로 본다."""
    try:
        db = sqlite3.connect(f"file:{home / 'saturn.db'}?mode=ro", uri=True, timeout=5)
        inputs = db.execute("select state, accepted_at from inputs order by id").fetchall()
        runs = db.execute("select ended_at from runs order by id").fetchall()
        db.close()
    except sqlite3.Error:
        return {"inputs": [], "runs": []}
    return {"inputs": inputs, "runs": [r[0] for r in runs]}


def finished(state: dict) -> bool:
    terminal = ("Applied", "Rejected", "Cancelled")
    return bool(state["inputs"]) and all(i[0] in terminal for i in state["inputs"]) and bool(state["runs"]) and all(r is not None for r in state["runs"])


def run_trial(run_id: str, tid: int, task: dict, arm: str, shadow: bool, key: str) -> dict:
    spec = ARMS[arm]
    home, practice = trial_dirs(tid)
    if home.exists() or practice.exists():
        raise RuntimeError("trial folder already exists: " + str(home))
    home.mkdir(parents=True)
    tasks.materialize(task, practice)
    subprocess.run(["git", "init", "-q"], cwd=practice, check=True)
    subprocess.run(["git", "add", "-A"], cwd=practice, check=True)
    subprocess.run(["git", "-c", "user.name=exp", "-c", "user.email=exp@example.invalid", "commit", "-qm", "start"], cwd=practice, check=True)
    if shadow:
        (home / "config.toml").write_text("[router.shadow]\nmodel_selection = true\n", encoding="utf-8")
    flags = ["-c", f"model.mode={spec['mode']}", "-c", f"model.candidates={json.dumps(spec['candidates'])}",
             "-c", "permission.mode=edit", "-c", 'permission.shell="allow"']
    if spec["mode"] == "manual":
        flags += ["-c", f"model.default={json.dumps(spec['model'])}"]
    session = f"t{tid:03d}"
    # tmux 서버는 첫 세션의 환경을 쓰므로 세션마다 환경을 셸 안에서 정한다. 키는 인자에 넣지 않고 같은 셸 안에서 키체인에서 읽는다.
    shell = (
        f"export SATURN_HOME={shlex.quote(str(home))}; "
        f"export SATURN_KEY=\"$(security find-generic-password -s {KEYCHAIN_SERVICE} -w)\"; "
        f"export PATH={shlex.quote(str(SATURN_BIN))}:$PATH; exec saturn " + " ".join(shlex.quote(f) for f in flags)
    )
    # 개발 구간에서 haiku가 작업 폴더가 아닌 바깥 저장소 경로를 읽으려다 허가 창에 멈춘 시도가 많아(12중 10) 폴더 경로를 한 문장으로 알린다
    sent = f"작업 폴더는 {practice} 이다. " + task["prompt"]
    note = []
    status = "ok"
    submitted = None
    subprocess.run([*TMUX, "new-session", "-d", "-s", session, "-x", "160", "-y", "50", "-c", str(practice), f"zsh -f -c {shlex.quote(shell)}"], check=True)
    try:
        if spec["mode"] == "auto":
            if not wait_for(session, WINDOW_TITLE, 90):
                status, note = "failed", ["default window did not open"]
            elif not pick_default(session, spec["model"]):
                status, note = "failed", ["default pick failed"]
        elif not wait_for(session, "Router jev", 60):
            status, note = "failed", ["screen did not open"]
        if status == "ok":
            time.sleep(1)
            tmux("send-keys", "-t", session, "-l", sent)
            tmux("send-keys", "-t", session, "Enter")
            submitted = time.time()
            deadline = submitted + TRIAL_TIMEOUT_S
            while True:
                time.sleep(2)
                state = db_state(home)
                if finished(state):
                    break
                if "Allow once" in pane(session):
                    status, note = "failed", ["permission prompt blocked the run"]
                    break
                if time.time() > deadline:
                    status, note = "delivery_unknown", ["timeout"]
                    break
        screen = pane(session)
    finally:
        tmux("kill-session", "-t", session)
    stopped = stop_engine(home)
    tables = dump_db(home) if (home / "saturn.db").exists() else {}
    inputs = tables.get("inputs")
    if status == "ok" and (not isinstance(inputs, list) or len(inputs) != 1 or inputs[0]["text"] != sent):
        status, note = "failed", note + ["input record does not match the sent prompt"]
    diff = subprocess.run(["git", "diff", "HEAD", "--stat", "--patch"], cwd=practice, capture_output=True, text=True).stdout
    ref = RUNTIME / "r" / task["task_id"]
    if not ref.exists():
        tasks.materialize(task, ref, solution=True)
    check = tasks.grade(task, practice, ref)
    return dict(
        run_id=run_id, tid=tid, task_id=task["task_id"], arm=arm, arm_spec=spec, shadow=shadow, binary_commit=binary_commit(),
        flags=flags, sent_prompt=sent, submitted_at=submitted, status=status, notes=note, engines_stopped=stopped,
        screen=screen.replace(key, "[key]"), tables=tables, diff=diff, check=check,
    )


def binary_commit() -> str:
    repo = SATURN_BIN.parents[1]
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True).stdout.strip()
