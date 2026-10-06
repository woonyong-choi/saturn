"""실제 provider 양방향 인계의 작은 진단. 사용자 홈을 쓰지 않고 원자료를 .local에 둔다."""

from __future__ import annotations

import hashlib
import http.server
import json
import os
import sqlite3
import threading
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
from engine_client import Client, Engine  # noqa: E402

RUN = ROOT / ".local/e2e/540-smoke" / f"{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{os.getpid()}"
MODELS = {"claude": "haiku", "codex": "gpt-5.6-luna"}


class Router(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        size = int(self.headers.get("Content-Length", 0))
        request = json.loads(self.rfile.read(size))
        answers = {}
        for name, question in request["questions"].items():
            kind = question["type"]
            if kind == "noul":
                answers[name] = {"type": "noul", "noul": 0.95}
            elif kind == "choice":
                names = list(question["criteria"])
                answers[name] = {"type": "choice", "choice": names[0], "probabilities": {x: (1.0 if x == names[0] else 0.0) for x in names}}
            elif kind == "score":
                names = list(question["criteria"])
                answers[name] = {"type": "score", "score": 0.0, "probabilities": {str(i): (1.0 if i == 0 else 0.0) for i in range(len(names))}}
        data = json.dumps({"model": request["model"], "answers": answers, "usage": {"input_tokens": 0, "output_tokens": 0}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def make_engine_wrapper(endpoint: str) -> Path:
    path = RUN / "test-engine"
    path.write_text(f'#!/bin/sh\nexec "{ROOT / "target/debug/saturn-engine"}" -c router.mode=saturn -c router.local.endpoint={endpoint} "$@"\n')
    path.chmod(0o700)
    return path


def sha(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def collect_db(home: Path, chat: int) -> dict:
    db = sqlite3.connect(f"file:{home / 'saturn.db'}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    def rows(query: str):
        return [dict(row) for row in db.execute(query, (chat,))]
    result = {
        "packets": rows("SELECT * FROM handoff_packets WHERE chat_id=? ORDER BY id"),
        "items": rows("SELECT i.* FROM handoff_packet_items i JOIN handoff_packets p ON p.id=i.packet_id WHERE p.chat_id=? ORDER BY i.packet_id, i.rowid"),
        "sessions": rows("SELECT id,provider,state FROM sessions WHERE chat_id=? ORDER BY id"),
        "events": rows("SELECT seq,body FROM events WHERE chat_id=? ORDER BY seq"),
    }
    db.close()
    return result


def run(source: str, target: str, binary: Path) -> dict:
    path = RUN / f"{source}-to-{target}"
    work, home_dir = path / "work", path / "home"
    home = Path(f"/tmp/sat540-{hashlib.sha256(str(RUN).encode()).hexdigest()[:8]}-{source[:2]}-{target[:2]}")
    work.mkdir(parents=True, exist_ok=True)
    home_dir.mkdir(parents=True, exist_ok=True)
    if not home.exists():
        home.symlink_to(home_dir, target_is_directory=True)
    elif home.resolve() != home_dir.resolve():
        raise RuntimeError(f"short engine path already points elsewhere: {home}")
    engine = Engine(binary, home, path / "engine.log", env=dict(os.environ, SATURN_HOME=str(home), SATURN_PACKET_CAPTURE="1"))
    client = Client(engine.sock_path)
    record = {"source": source, "target": target, "inputs": []}
    try:
        chat = client.attach(work, [])
        client.set_model(chat, source, MODELS[source])
        for prompt in [
            "For this diagnostic, reply with exactly ACK-ONE. Do not use tools.",
            "Correction for this chat: the header is X-Route-Key instead of X-Req-Id. Reply with exactly ACK-TWO. Do not use tools.",
            "Read request one: acknowledge this line only with ACK-THREE. Do not use tools.",
            "Read request two: acknowledge this line only with ACK-FOUR. Do not use tools.",
            "Read request three: acknowledge this line only with ACK-FIVE. Do not use tools.",
            "Read request four: acknowledge this line only with ACK-SIX. Do not use tools.",
        ]:
            out = client.run_input(chat, prompt, allow=lambda _: False, timeout=120)
            record["inputs"].append({"text": prompt, "status": out["status"], "error": out.get("error"), "tasks": out.get("tasks"), "notes": out.get("notes", [])})
            if out["status"] != "ok" or any(state != "Done" for state in out.get("tasks", {}).values()):
                break
        if len(record["inputs"]) == 6 and all(item["status"] == "ok" and all(state == "Done" for state in item["tasks"].values()) for item in record["inputs"]):
            client.set_model(chat, target, MODELS[target])
            prompt = "Which header should this chat use now? Reply with only the header name. Do not use tools."
            out = client.run_input(chat, prompt, allow=lambda _: False, timeout=180)
            record["target_input"] = {"text": prompt, "status": out["status"], "error": out.get("error"), "tasks": out.get("tasks"), "notes": out.get("notes", [])}
        record["db"] = collect_db(home, chat)
        record["captures"] = [json.loads(p.read_text()) for p in sorted((home / "packet-capture").glob("*.json"))]
    except Exception as error:
        record["error"] = repr(error)
    finally:
        client.close()
        engine.stop()
    (path / "raw.json").write_text(json.dumps(record, ensure_ascii=False, indent=2))
    packets = record.get("db", {}).get("packets", [])
    result = {
        "source": source,
        "target": target,
        "source_statuses": [item["status"] for item in record["inputs"]],
        "target_status": record.get("target_input", {}).get("status"),
        "packets": [
            {"id": p["id"], "state": p["state"], "hash": p["body_hash"], "kind": p["kind"]}
            for p in packets
        ],
        "protected_row_count": sum(i["body_hash"] is not None for i in record.get("db", {}).get("items", [])),
        "target_text": "".join(
            note["params"]["event"]["Text"]["text"]
            for note in record.get("target_input", {}).get("notes", [])
            if note.get("method") == "TaskEvent" and "Text" in note.get("params", {}).get("event", {})
        ),
        "capture_count": len(record.get("captures", [])),
        "capture_hashes_match": bool(record.get("captures")) and all(c["body_hash"] == sha(c["body"]) and any(p["id"] == c["packet_id"] and p["body_hash"] == c["body_hash"] for p in packets) for c in record.get("captures", [])),
        "error": record.get("error"),
    }
    (path / "summary.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
    return result


if __name__ == "__main__":
    os.umask(0o077)
    RUN.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Router)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        binary = make_engine_wrapper(f"http://127.0.0.1:{server.server_port}")
        results = [run("codex", "claude", binary), run("claude", "codex", binary)]
        print(json.dumps({"run": str(RUN), "results": results}, ensure_ascii=False, indent=2))
    finally:
        server.shutdown()
