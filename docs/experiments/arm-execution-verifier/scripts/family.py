"""계열 대화 재생(#540 보존 우선 비교): online.py의 스냅샷·시험 경로(Engine, Client, dump_db)를 계열 fixture에 맞춰 쓴다.

source provider가 fixture의 사용자 입력을 그대로 받아 기록을 만들고, 시험마다 그 스냅샷을 복원해 다른 provider로 바꾼 뒤 후속 요청 세 개를 보낸다.
보낼 패킷 본문은 격리 홈에서 engine이 남기는 캡처로 받는다. 기존 online.py의 동작은 바꾸지 않는다.
"""

from __future__ import annotations

import json
import os
import shutil
import time
from pathlib import Path

import online
from engine_client import ATTACH_ENV, Client, Engine

CAPTURE_ENV = "SATURN_PACKET_CAPTURE"
LOG_FILE = "build.log"
SOCKET_LIMIT = 100  # macOS unix 소켓 경로 길이 제한(104)에서 여유를 둔다
TURN_TIMEOUT_S = 300  # preserve.CAPS["timeout_s"]와 같다


class ReplayError(RuntimeError):
    """계열 대화를 결정적으로 재생하지 못했다. 대체 값을 만들지 않고 사유를 그대로 올린다."""


def socket_path_problem(run: Path) -> str | None:
    longest = run / "s5a" / "engine.sock"
    if len(str(longest)) >= SOCKET_LIMIT:
        return f"socket path too long ({len(str(longest))} >= {SOCKET_LIMIT}): {longest}"
    return None


def capture_env(home: Path) -> dict:
    """격리한 홈에서만 보낼 패킷 본문 캡처를 켠다. SATURN_HOME은 engine의 --home과 같은 값이어야 켜진다."""
    env = dict(os.environ, SATURN_HOME=str(home))
    env[CAPTURE_ENV] = "1"
    return env


def allow_log(summary: str) -> bool:
    return LOG_FILE in summary


def log_read_requested(fix: dict) -> bool:
    """고정 입력에 로그 읽기 요청이 있는지. 에이전트가 읽지 않으면 로그가 도구 결과로 남지 않아 재생이 fixture와 달라진다.
    합성 도구 결과로 메우지 않으므로, 요청이 없는 계열은 수집 전에 지원하지 않는 것으로 걸러낸다."""
    return any(LOG_FILE in user or "빌드 로그" in user for user in fix["visible"]["users"])


def build_snapshot(source: str, fix: dict, run: Path, label: str) -> dict:
    """source provider가 fixture의 사용자 입력을 차례로 받아 기록을 만든다. 로그 파일은 에이전트가 직접 읽게 둔다.
    입력이 모두 적용되지 않았거나 로그 내용이 도구 결과로 기록되지 않았으면 재생 실패다."""
    visible = fix["visible"]
    if not log_read_requested(fix):
        raise ReplayError("the fixture inputs never ask to read the build log")
    home, work = run / f"s{label}", run / f"w{label}"
    if home.exists() or work.exists():
        raise ReplayError("attempt home already exists; a started attempt is never repeated or deleted: " + str(home))
    work.mkdir(parents=True)
    (work / LOG_FILE).write_text(visible["tool_results"][0] + "\n")
    engine = Engine(online.BINARY, home, run / f"s{label}.engine.log")
    try:
        client = Client(engine.sock_path)
        chat = client.attach(work, [])
        client.set_model(chat, source, online.PROVIDERS[source])
        for k, text in enumerate(visible["users"]):
            out = client.run_input(chat, text, allow_log, timeout=TURN_TIMEOUT_S)
            if out["status"] != "ok" or any(v != "Done" for v in out["tasks"].values()):
                raise ReplayError(f"source turn {k} did not finish: {out['status']} {out.get('tasks')}")
        decisions = client.decisions
    finally:
        engine.stop()
    (work / LOG_FILE).unlink()
    db = online.dump_db(home / "saturn.db", chat)
    if [i["text"] for i in db["inputs"]] != visible["users"] or any(i["state"] != "Applied" for i in db["inputs"]):
        raise ReplayError("source inputs differ from the fixture or were not applied")
    probe = next(line for line in visible["tool_results"][0].splitlines() if line.startswith("build log for"))
    if not any(probe in e["body"] and e["body"].startswith('{"ToolResult"') for e in db["events"]):
        raise ReplayError("the build log was not read into a tool result; replay is not equal to the fixture")
    raw_path = run / f"s{label}.raw.json"
    fd = os.open(raw_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as out:
        out.write(json.dumps(dict(decisions=decisions, db=db), ensure_ascii=False))
        out.flush()
        os.fsync(out.fileno())
    return dict(chat=chat, label=label, calls=call_counts(db, 0),
                source_packet_max_id=max((p["id"] for p in db["packets"]), default=0))


def call_counts(db: dict, since_ms: int) -> dict:
    """기록된 입력 실행과 패킷 시도로 센 provider 전송 수. 누적 usage 행의 개수를 호출 수로 쓰지 않는다."""
    runs, packets, judgments = db.get("runs"), db.get("packets"), db.get("judgments")
    inputs = None if runs is None or packets is None else (
        len({r["input_id"] for r in runs if r["input_id"] is not None and r["started_at"] >= since_ms})
        + sum(p["created_at"] >= since_ms for p in packets)
    )
    return dict(inputs=inputs,
                jev=None if judgments is None else sum(j["started_at"] >= since_ms for j in judgments))


def read_captures(home: Path) -> list:
    folder = home / "packet-capture"
    if not folder.is_dir():
        return []
    return [json.loads(p.read_text()) for p in sorted(folder.glob("*.json"), key=lambda p: int(p.stem))]


def reply_text(notes: list) -> str:
    """한 입력의 답. 패킷 턴의 답(PacketReply)은 버리고 그 뒤의 본문 글만 모은다."""
    text = ""
    for n in notes:
        ev = n.get("params", {}).get("event", {}) if n.get("method") == "TaskEvent" else {}
        if "PacketReply" in ev:
            text = ""
        elif "Text" in ev and ev["Text"].get("subagent") is None:
            text += ev["Text"]["text"]
    return text.strip()


def run_trial(cell: dict, fix: dict, snap: dict, overrides: list, run: Path, tag: str = "t") -> dict:
    """스냅샷을 복원해 target provider로 후속 요청 세 개를 차례로 보낸다. 앞 요청이 끝나지 않으면 나머지를 보내지 않고 재전송하지 않는다."""
    label, target = snap["label"], cell["target"]
    home, work = run / tag, run / f"w{label}"
    shutil.copytree(run / f"s{label}", home, symlinks=True)  # 이미 있으면 실패한다. 시도 홈은 지우거나 덮어쓰지 않는다
    engine = Engine(online.BINARY, home, run / f"{tag}.engine.log", env=capture_env(home))
    record = dict(name=cell["cell_id"], source=cell["source"], target=target, arm=cell["arm"], rep=0, hint=False, overrides=overrides,
                  source_packet_max_id=snap["source_packet_max_id"])
    answers: list = []
    try:
        client = Client(engine.sock_path)
        env = [[k, os.environ[k]] for k in ATTACH_ENV if k in os.environ and k != "PATH"]
        env.append(["PATH", f"{online.BINARY.parent}:{os.environ['PATH']}"])
        client.call("Attach", {"chat": snap["chat"], "workdir": str(work), "env": env, "overrides": overrides, "add_dirs": []})
        client.set_model(snap["chat"], target, online.PROVIDERS[target])
        record["started_at"] = online.now()
        t0 = record["t0"] = time.time()
        status, tasks, notes = "ok", {}, []
        for text in fix["visible"]["followups"]:
            out = client.run_input(snap["chat"], text, lambda summary: False, timeout=TURN_TIMEOUT_S)
            notes += out.get("notes", [])
            tasks.update(out.get("tasks") or {})
            answers.append(reply_text(out.get("notes", [])))
            if out["status"] != "ok" or any(v == "Failed" for v in (out.get("tasks") or {}).values()):
                status = out["status"] if out["status"] != "ok" else "failed"
                break
        record.update(latency_s=round(time.time() - t0, 6), ended_at=online.now(), status=status, tasks=tasks, notes=notes, decisions=client.decisions)
    finally:
        engine.stop()
    record["answers"] = answers
    record["workdir_files"] = sorted(p.name for p in work.iterdir())
    record["captures"] = read_captures(home)
    record["sent_body"] = record["captures"][-1]["body"] if record["captures"] else None
    record["db"] = online.dump_db(home / "saturn.db", snap["chat"])
    record["calls"] = call_counts(record["db"], int(t0 * 1000))
    shutil.rmtree(home, ignore_errors=True)  # 끝까지 마친 시험 홈만 지운다. 중단된 시도의 홈은 남는다
    return record
