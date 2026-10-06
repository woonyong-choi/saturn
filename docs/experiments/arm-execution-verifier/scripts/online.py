"""온라인 단계: 실제 engine 경로로 같은 과제를 조건별로 돌리고 원자료를 .runtime에 남긴다.

source provider가 블록 파일 12개를 각각 읽어 기록을 만들고(스냅샷), 시험마다 그 스냅샷을 복원해
다른 provider로 바꾼 뒤 과제를 보낸다. 조건은 engine의 패킷 설정만 바꾼다. 재시도하지 않는다.
"""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

import sample
from engine_client import ATTACH_ENV, Client, Engine

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
RUN = ROOT / ".runtime/ar"  # 소켓 경로 길이 제한 때문에 짧게 둔다. git 무시 폴더
BINARY = Path(os.environ.get("ARM_ENGINE_BIN", ""))
BLOCK_CHARS = 5600
FAMILY_TASK = "x000a"
PROVIDERS = {"claude": "haiku", "codex": "gpt-5.6-luna"}
ARMS = ("full", "code", "jev")  # code = engine의 RRF 순위. llm은 engine 연결이 없어 이 경로에 없다
REPS = 3
# 목표 패킷 한도는 약 5000토큰(블록 3개 안팎). 한도 = 창 x safety% / 10
SAFETY = {"claude": "5", "codex": "18"}
FULL_SAFETY = "100"
LIMITS = {"claude": 40, "codex": 40, "router": 60}


def case() -> dict:
    c = next(c for c in sample.build() if c["task_id"] == FAMILY_TASK)
    blocks = []
    for b in c["blocks"]:
        fact = b["text"].split(" archival padding.")[0]
        # 근거는 블록 끝에만 둔다. 패킷의 발췌(앞 300글자)만으로는 답할 수 없다
        text = ("Archive entry." + " archival padding." * 400)[: BLOCK_CHARS - len(fact) - 1] + "\n" + fact
        blocks.append({"id": b["id"], "text": text})
    return dict(c, blocks=blocks)


def write_files(c: dict, work: Path) -> None:
    work.mkdir(parents=True, exist_ok=True)
    for b in c["blocks"]:
        (work / f"rec-{b['id']}.txt").write_text(b["text"] + "\n")


def remove_files(c: dict, work: Path) -> None:
    for b in c["blocks"]:
        (work / f"rec-{b['id']}.txt").unlink()


def allow_read(summary: str) -> bool:
    return "cat rec-" in summary


def allow_evidence(summary: str) -> bool:
    return "evidence" in summary and ("search" in summary or "read" in summary)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def build_snapshot(source: str, c: dict) -> dict:
    """source provider가 파일을 하나씩 읽게 해 기록을 만들고 홈을 스냅샷으로 남긴다."""
    home, work = RUN / f"snap-{source}", RUN / f"work-{source}"
    shutil.rmtree(home, ignore_errors=True)
    shutil.rmtree(work, ignore_errors=True)
    write_files(c, work)
    engine = Engine(BINARY, home, RUN / f"snap-{source}.engine.log")
    try:
        client = Client(engine.sock_path)
        chat = client.attach(work, [])
        client.set_model(chat, source, PROVIDERS[source])
        files = " ".join(f"rec-{b['id']}.txt" for b in c["blocks"])
        out = client.run_input(
            chat,
            f"Run cat on each of these files as its own separate shell command: {files}. Then reply done.",
            allow_read,
        )
        if out["status"] != "ok":
            raise RuntimeError("snapshot build failed: " + out["status"])
        (RUN / f"snap-{source}.raw.json").write_text(
            json.dumps({"notes": out["notes"], "decisions": client.decisions}, ensure_ascii=False)
        )
    finally:
        engine.stop()
    remove_files(c, work)
    return {"chat": chat}


HINT = " Omitted records can be listed with `saturn evidence search <query>` and read with `saturn evidence read <number>`."


def run_trial(source: str, target: str, arm: str, rep: int, c: dict, chat: int, hint: bool = False) -> dict:
    """스냅샷을 복원해 target provider로 과제를 보낸다. 원응답과 기록 저장소 행을 모두 남긴다."""
    name = f"{c['task_id']}-{arm}-{target}{'-hint' if hint else ''}-r{rep}"
    home = RUN / "trial-home"
    shutil.rmtree(home, ignore_errors=True)
    shutil.copytree(RUN / f"snap-{source}", home, symlinks=True)
    overrides = [["context.evidence.lookup", "true"]]
    safety = FULL_SAFETY if arm == "full" else SAFETY[target]
    overrides.append(["context.safety_percent", safety])
    if arm == "jev":
        overrides.append(["context.select.packet", "jev"])
    engine = Engine(BINARY, home, RUN / "trial.engine.log")
    record: dict = {"name": name, "source": source, "target": target, "arm": arm, "rep": rep, "hint": hint, "overrides": overrides}
    try:
        client = Client(engine.sock_path)
        # 실제 사용자 환경처럼 saturn 실행 파일이 PATH에 있어야 에이전트가 근거 조회를 쓸 수 있다
        env = [[k, os.environ[k]] for k in ATTACH_ENV if k in os.environ and k != "PATH"]
        env.append(["PATH", f"{BINARY.parent}:{os.environ['PATH']}"])
        client.call("Attach", {"chat": chat, "workdir": str(RUN / f"work-{source}"), "env": env, "overrides": overrides, "add_dirs": []})
        client.set_model(chat, target, PROVIDERS[target])
        record["started_at"] = now()
        t0 = record["t0"] = time.time()
        out = client.run_input(chat, c["task"] + (HINT if hint else ""), allow_evidence)
        record["latency_s"] = round(time.time() - t0, 6)
        record["ended_at"] = now()
        record["status"] = out["status"]
        record["tasks"] = out.get("tasks")
        record["notes"] = out.get("notes", [])
        record["decisions"] = client.decisions
    finally:
        engine.stop()
    record["db"] = dump_db(home / "saturn.db", chat)
    shutil.rmtree(home, ignore_errors=True)
    return record


def dump_db(path: Path, chat: int) -> dict:
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row

    def rows(sql: str, *args) -> list[dict]:
        return [dict(r) for r in db.execute(sql, args)]

    return {
        "packets": rows("select * from handoff_packets where chat_id=? order by id", chat),
        "items": rows("select i.* from handoff_packet_items i join handoff_packets p on p.id=i.packet_id where p.chat_id=? order by i.packet_id, i.rowid", chat),
        "lookups": rows("select * from evidence_lookups where chat_id=? order by id", chat),
        "usage": rows("select * from usage where chat_id=? order by id", chat),
        "runs": rows("select id,input_id,task_id,session_id,provider,started_at,ended_at,end_kind from runs where chat_id=? order by id", chat),
        "judgments": rows("select id,input_id,started_at,method,router,model,reported_model,answers,fallbacks,input_tokens,output_tokens,elapsed_ms,outcome from judgments where chat_id=? order by id", chat),
        "sessions": rows("select id,provider,provider_session,model from sessions where chat_id=? order by id", chat),
        "events": rows("select seq,run_id,body from events where chat_id=? order by seq", chat),
        "inputs": rows("select id,text,state from inputs where chat_id=? order by id", chat),
    }
