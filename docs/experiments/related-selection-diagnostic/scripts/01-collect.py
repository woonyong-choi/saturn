#!/usr/bin/env python3
"""실제 engine에서 같은 source 기록을 세 선별 조건으로 재생한다."""

from __future__ import annotations

import hashlib
import json
import os
import random
import shutil
import sqlite3
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
BASE = HERE.parent
ROOT = BASE.parents[2]
STORAGE = ROOT / ".runtime" / "related-selection-diagnostic"
RUNTIME = Path("/tmp/srd541")
ENGINE_CLIENT = ROOT / "docs" / "experiments" / "arm-execution-verifier" / "scripts"
sys.path.insert(0, str(ENGINE_CLIENT))
from engine_client import Client, Engine  # noqa: E402

SEED = 541202
KINDS = ("exact", "stale", "synonym", "middle")
ARMS = ("off", "rank", "jev")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path: Path, data: dict, secret: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    body = json.dumps(data, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    path.write_text(body.replace(secret, "[secret]"), encoding="utf-8")


def scenario(kind: str, seed: int) -> dict:
    project = f"Project Meridian-{kind}-{seed}"
    answer = 731 + seed * 17 + KINDS.index(kind) * 100
    question = f"For {project}, what is the current cache eviction cap? Reply with only the integer."
    if kind == "synonym":
        question = f"For {project}, how many cache slots may be retained? Reply with only the integer."
    return dict(kind=kind, seed=seed, project=project, answer=answer, question=question)


def records(spec: dict) -> list[str]:
    kind, project, answer = spec["kind"], spec["project"], spec["answer"]
    target = 3 + (spec["seed"] * 3 + KINDS.index(kind)) % 8
    entries = []
    for i in range(12):
        other = f"Project Foxtrot-{spec['seed']}-{i}"
        body = f"{other}: current cache eviction cap is {300 + i * 11} entries."
        if i == target:
            if kind == "synonym":
                body = f"{project}: maximum number of retained cache slots is {answer}."
            elif kind == "middle":
                padding = "Historical notes for unrelated settings. " * 52
                body = padding + f"\n{project}: current cache eviction cap is {answer} entries.\n" + padding
            elif kind == "stale":
                body = f"{project}: correction dated release 2, current cache eviction cap is {answer} entries. Earlier release 1 value is obsolete."
            else:
                body = f"{project}: current cache eviction cap is {answer} entries."
        if kind == "stale" and i == 1:
            body = f"{project}: release 1 cache eviction cap was {answer - 60} entries. This record is obsolete."
        entries.append(body)
    return entries


def query(db_path: Path, chat: int, since: int) -> dict:
    db = sqlite3.connect(db_path)
    db.row_factory = sqlite3.Row

    def rows(statement: str) -> list[dict]:
        return [dict(row) for row in db.execute(statement, (chat, since))]

    result = {
        "packets": rows("select id,kind,provider,state,body_hash,body_bytes,estimated_tokens from handoff_packets where chat_id=? and created_at>=? order by id"),
        "items": rows("select zone,ref_id,selector,form,reason,body_hash from handoff_packet_items where packet_id in (select id from handoff_packets where chat_id=? and created_at>=?) order by packet_id,rowid"),
        "judgments": rows("select id,question_sets,answers,fallbacks,input_tokens,output_tokens,elapsed_ms,outcome from judgments where chat_id=? and started_at>=? order by id"),
        "usage": rows("select scope,input_tokens,cache_write_tokens,cache_read_tokens,output_tokens,at from usage where chat_id=? and at>=? order by at"),
        "lookups": rows("select kind,outcome,record_id from evidence_lookups where chat_id=? and created_at>=? order by created_at"),
    }
    db.close()
    return result


def final_text(notes: list[dict]) -> str:
    texts = []
    for note in notes:
        event = note.get("params", {}).get("event", {})
        if note.get("method") == "TaskEvent" and "Text" in event:
            texts.append(event["Text"].get("text", ""))
    return texts[-1].strip() if texts else ""


def make_source(spec: dict, run: Path, binary: Path, template: Path, secret: str) -> int | None:
    case = f"{spec['kind']}-{spec['seed']}"
    work = run / case / "work"
    home = run / case / "source-home"
    work.mkdir(parents=True)
    home.mkdir()
    shutil.copy2(template, home / "config.toml")
    names = []
    expected_outputs = records(spec)
    for ordinal, body in enumerate(expected_outputs):
        name = f"record-{ordinal:02}.txt"
        (work / name).write_text(body + "\n", encoding="utf-8")
        names.append(name)
    engine = Engine(binary, home, run / case / "source-engine.log")
    try:
        client = Client(engine.sock_path)
        chat = client.attach(work, [["context.select.related", "off"]])
        client.set_model(chat, "codex", "gpt-5.6-luna")
        prompt = "Run cat on each file as a separate shell command: " + " ".join(names) + ". Then reply only done."
        outcome = client.run_input(chat, prompt, lambda value: "cat record-" in value, timeout=300)
        db = sqlite3.connect(home / "saturn.db")
        tool_outputs = []
        for (body,) in db.execute("select body from events where chat_id=? order by seq", (chat,)):
            tool_result = json.loads(body).get("ToolResult")
            if tool_result and tool_result.get("exit_code") == 0:
                tool_outputs.append(tool_result.get("output", ""))
        db.close()
        complete = all(any(expected in output for output in tool_outputs) for expected in expected_outputs)
        save(run / case / "source.json", dict(spec=spec, status=outcome["status"], notes=outcome["notes"], chat=chat, source_db_sha256=digest(home / "saturn.db"), tool_results=len(tool_outputs), source_complete=complete), secret)
    finally:
        engine.stop()
    for name in names:
        (work / name).unlink()
    return chat if outcome["status"] == "ok" and complete else None


def trial(spec: dict, run: Path, binary: Path, chat: int, arm: str, secret: str) -> None:
    case = f"{spec['kind']}-{spec['seed']}"
    folder = run / case
    home = folder / f"{arm}-home"
    shutil.copytree(folder / "source-home", home)
    engine = Engine(binary, home, folder / f"{arm}-engine.log")
    started = int(time.time() * 1000)
    begin = time.monotonic()
    try:
        client = Client(engine.sock_path)
        overrides = [["context.select.related", arm], ["context.evidence.lookup", "true"], ["context.safety_percent", "5"]]
        env = [[name, os.environ[name]] for name in ("HOME", "USER", "SHELL", "LANG", "TERM") if name in os.environ]
        env.append(["PATH", str(binary.parent) + ":" + os.environ["PATH"]])
        client.call("Attach", {"chat": chat, "workdir": str(folder / "work"), "env": env, "overrides": overrides, "add_dirs": []})
        client.set_model(chat, "claude", "sonnet")
        prompt = spec["question"] + " Use earlier records. If needed, run `saturn evidence search` and `saturn evidence read` to recover evidence."
        outcome = client.run_input(chat, prompt, lambda value: "evidence" in value, timeout=240)
        elapsed = time.monotonic() - begin
        row = dict(case=case, arm=arm, status=outcome["status"], answer=final_text(outcome["notes"]), expected=spec["answer"], latency_s=elapsed, notes=outcome["notes"], decisions=client.decisions, db=query(home / "saturn.db", chat, started))
    finally:
        engine.stop()
    save(folder / f"{arm}.json", row, secret)


def main() -> None:
    secret = Path(os.environ["SATURN_KEY_FILE"]).read_text(encoding="utf-8").strip()
    os.environ["SATURN_KEY"] = secret
    binary = Path(os.environ["SATURN_ENGINE_BIN"]).resolve()
    template = Path(os.environ["SATURN_CONFIG_TEMPLATE"]).resolve()
    if not binary.is_file() or not template.is_file() or not secret:
        raise RuntimeError("missing experiment input")
    os.umask(0o077)
    STORAGE.mkdir(parents=True, exist_ok=True)
    if RUNTIME.is_symlink():
        if RUNTIME.resolve() != STORAGE.resolve():
            raise RuntimeError("runtime symlink points elsewhere")
    elif RUNTIME.exists():
        raise RuntimeError("runtime path already occupied")
    else:
        RUNTIME.symlink_to(STORAGE, target_is_directory=True)
    commit = subprocess.check_output(["git", "rev-parse", "--short=7", "HEAD"], cwd=ROOT, text=True).strip()
    run_id = os.environ.get("SATURN_RUN_ID") or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + commit
    run = RUNTIME / run_id
    run.mkdir(parents=True, exist_ok=False)
    save(run / "manifest.json", dict(run_id=run_id, binary_sha256=digest(binary), template_sha256=digest(template), source_branch="feat/600-related-memory", seed=SEED, cases=[scenario(kind, seed) for kind in KINDS for seed in range(3)]), secret)
    rng = random.Random(SEED)
    for kind in KINDS:
        for seed in range(3):
            spec = scenario(kind, seed)
            chat = make_source(spec, run, binary, template, secret)
            if chat is None:
                continue
            order = list(ARMS)
            rng.shuffle(order)
            for arm in order:
                trial(spec, run, binary, chat, arm, secret)
            print(f"{kind}-{seed}: complete", flush=True)
    print(str(run), flush=True)


if __name__ == "__main__":
    main()
