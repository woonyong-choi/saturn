"""사전 봉인한 독립 패킷 선별 실험을 실제 engine과 두 CLI에서 수집한다."""

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
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[2]
RUN = ROOT / ".runtime/sc"
BINARY = ROOT / "target/debug/saturn-engine"
sys.path.insert(0, str(HERE.parent / "arm-execution-verifier/scripts"))
from engine_client import ATTACH_ENV, Client, Engine  # noqa: E402
from family import reply_text  # noqa: E402

SEED = 54026006
NAMES = ("build.log", "archive.log", "deploy.log")
OVERRIDES = [["context.safety_percent", "5"], ["provider.claude.context.window", "100000"]]
PROMPT = "For the current release, what timeout was recorded? Number only. Do not use tools."


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def save(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w") as f:
        json.dump(value, f, ensure_ascii=False, sort_keys=True, indent=2)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    tmp.replace(path)


def source_files(index: int) -> tuple[dict[str, str], int, str]:
    rng = random.Random(SEED + index)
    current_name = NAMES[index % 3]
    numbers = rng.sample(range(18, 89), 3)
    current = numbers[index % 3]
    files = {}
    for i, name in enumerate(NAMES):
        release = "current" if name == current_name else "old"
        head = f"FILE={name} RELEASE={release} TIMEOUT={numbers[i]} seconds\n"
        files[name] = head + (f"archived detail number {i} for {name}. " * 430)
    return files, current, current_name


def db_rows(path: Path, chat: int) -> dict:
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    def rows(table: str, where: str = "chat_id=?") -> list[dict]:
        return [dict(r) for r in db.execute(f"SELECT * FROM {table} WHERE {where} ORDER BY id", (chat,))]
    out = {"packets": rows("handoff_packets"), "usage": rows("usage"), "judgments": rows("judgments")}
    out["events"] = [dict(r) for r in db.execute("SELECT seq,body FROM events WHERE chat_id=? ORDER BY seq", (chat,))]
    out["items"] = [dict(r) for r in db.execute("SELECT i.* FROM handoff_packet_items i JOIN handoff_packets p ON p.id=i.packet_id WHERE p.chat_id=? ORDER BY i.packet_id,i.rowid", (chat,))]
    db.close()
    return out


def protocol_hashes() -> dict[str, str]:
    files = [HERE / "design.md", HERE / "run.sh", *sorted((HERE / "scripts").glob("*.py"))]
    return {str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in files}


def seal() -> None:
    if (RUN / "manifest.json").exists():
        raise SystemExit("이미 봉인됨")
    for path in protocol_hashes():
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", path], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "ls-files", "--error-unmatch", path], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    if not BINARY.is_file():
        raise SystemExit("engine binary missing")
    prices = json.loads((ROOT / ".runtime/preserve/prices.json").read_text())
    manifest = {
        "seed": SEED, "planned": 12, "protocol": protocol_hashes(), "binary_sha256": sha(BINARY.read_bytes()),
        "git_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "prices": prices, "models": {"source": "gpt-5.6-luna", "target": "claude-haiku-4-5-20251001", "jev": "jev-1.13.0"},
        "versions": {cmd: subprocess.check_output([cmd, "--version"], text=True, timeout=15).strip() for cmd in ("claude", "codex")},
        "order": {str(i): random.Random(SEED + i).sample(["R", "J"], 2) for i in range(12)},
        "fixtures": {str(i): {"current_name": source_files(i)[2], "file_hashes": {k: sha(v.encode()) for k, v in source_files(i)[0].items()}} for i in range(12)},
        "caps": {"provider_inputs": 120, "jev_calls": 120},
    }
    RUN.mkdir(mode=0o700, parents=True, exist_ok=True)
    save(RUN / "manifest.json", manifest)


def check_seal() -> dict:
    manifest = json.loads((RUN / "manifest.json").read_text())
    if manifest["protocol"] != protocol_hashes() or manifest["binary_sha256"] != sha(BINARY.read_bytes()):
        raise SystemExit("봉인 뒤 실행기 또는 바이너리 변경")
    return manifest


def journal(label: str) -> None:
    p = RUN / "ledger" / f"{label}.json"
    if p.exists():
        raise RuntimeError(f"attempt already journaled: {label}")
    save(p, {"attempt": label, "at_ns": time.time_ns()})


def source(index: int) -> tuple[int | None, bool]:
    label = f"s{index:02}"
    home, work = RUN / label / "home", RUN / label / "work"
    files, _, _ = source_files(index)
    home.mkdir(parents=True)
    work.mkdir(parents=True)
    for name, body in files.items():
        (work / name).write_text(body)
    env = dict(os.environ, SATURN_HOME=str(home))
    engine = Engine(BINARY, home, RUN / label / "engine.log", env=env)
    client = Client(engine.sock_path)
    chat = None
    outputs = []
    try:
        chat = client.attach(work, [])
        client.set_model(chat, "codex", "gpt-5.6-luna")
        for name in NAMES:
            journal(f"{label}-{name}")
            output = client.run_input(chat, f"Use a tool to read {name} completely. Reply with only READY after the read.", allow=lambda s: ".log" in s, timeout=300)
            outputs.append({"name": name, "status": output["status"], "tasks": output.get("tasks"), "notes": output.get("notes", [])})
            if output["status"] != "ok" or not output.get("tasks") or any(x != "Done" for x in output["tasks"].values()):
                break
    finally:
        client.close()
        engine.stop()
    db = db_rows(home / "saturn.db", chat) if chat else {}
    seen = {name: any(e["body"].startswith('{"ToolResult"') and f"FILE={name}" in e["body"] for e in db.get("events", [])) for name in NAMES}
    valid = len(outputs) == 3 and all(seen.values()) and all(o["status"] == "ok" and all(v == "Done" for v in o["tasks"].values()) for o in outputs)
    save(RUN / "raw" / f"{label}.json", {"chat": chat, "outputs": outputs, "seen": seen, "valid": valid, "db": db})
    return chat, valid


def target(index: int, chat: int, arm: str) -> None:
    label = f"t{index:02}{arm}"
    source_home = RUN / f"s{index:02}" / "home"
    home = RUN / label / "home"
    shutil.copytree(source_home, home, ignore=shutil.ignore_patterns("engine.sock", "*.lock", "packet-capture"))
    work = RUN / label / "work"
    work.mkdir(parents=True)
    overrides = [*OVERRIDES, *([["context.select.packet", "jev"]] if arm == "J" else [])]
    env = dict(os.environ, SATURN_HOME=str(home), SATURN_PACKET_CAPTURE="1")
    engine = Engine(BINARY, home, RUN / label / "engine.log", env=env)
    client = Client(engine.sock_path)
    output = {}
    t0 = time.monotonic()
    stamp_ms = int(time.time() * 1000)
    try:
        attach_env = [[k, os.environ[k]] for k in ATTACH_ENV if k in os.environ and k != "PATH"]
        attach_env.append(["PATH", f"{BINARY.parent}:{os.environ['PATH']}"])
        client.call("Attach", {"chat": chat, "workdir": str(work), "env": attach_env, "overrides": overrides, "add_dirs": []})
        client.set_model(chat, "claude", "haiku")
        journal(label)
        output = client.run_input(chat, PROMPT, allow=lambda _: False, timeout=300)
    finally:
        duration = time.monotonic() - t0
        client.close()
        engine.stop()
    db = db_rows(home / "saturn.db", chat)
    captures = [json.loads(p.read_text()) for p in sorted((home / "packet-capture").glob("*.json"))] if (home / "packet-capture").exists() else []
    save(RUN / "raw" / f"{label}.json", {"status": output.get("status"), "tasks": output.get("tasks"), "notes": output.get("notes", []),
        "answer": reply_text(output.get("notes", [])), "duration_s": duration, "started_at_ms": stamp_ms, "chat": chat,
        "overrides": overrides, "db": db, "captures": captures})


def collect() -> None:
    manifest = check_seal()
    os.environ["SATURN_KEY"] = (ROOT / ".runtime/router.key").read_text().strip()
    for i in range(manifest["planned"]):
        sfile = RUN / "raw" / f"s{i:02}.json"
        if not sfile.exists():
            if any((RUN / "ledger").glob(f"s{i:02}-*.json")):
                print(f"source {i}: unresolved attempt; skip", flush=True)
                continue
            chat, valid = source(i)
        else:
            src = json.loads(sfile.read_text())
            chat, valid = src["chat"], src["valid"]
        print(f"source {i}: {'valid' if valid else 'failed'}", flush=True)
        if not valid:
            continue
        for arm in manifest["order"][str(i)]:
            label = f"t{i:02}{arm}"
            if (RUN / "raw" / f"{label}.json").exists() or (RUN / "ledger" / f"{label}.json").exists():
                continue
            target(i, chat, arm)
            print(f"target {i} {arm}: collected", flush=True)


if __name__ == "__main__":
    os.umask(0o077)
    {"seal": seal, "collect": collect}[sys.argv[1]]()
