"""#577 기능 진단 R/J 대응 실행. 확인 평가 80쌍과 분리한다.

`preflight`는 외부 호출 없이 source와 코드 봉인을 검사한다. `collect`는
사전등록 커밋 뒤에만 사용한다. 실패·불명 전송은 재시도하지 않는다.
"""

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
RUN = ROOT / ".runtime/jr"
SOURCE = RUN / "s1"
EXPECTED = {
    "binary": "cc916075f893223178afc4bf6aee6839aa3545420dc8d4051ab3d9e064ae6405",
    "raw": "5fff75d94e6eef6aa9a9083074d4357a5db420c7cea71f49a504b20d5c469781",
    "db": "f9e6ce63d7c1abc9612b824d902cdc36ae4d1268e135aece264534581fcdc027",
    "home": "6d935ef7ef3a100662ec47dc7d9493dba8b6e828931218ad03bd5096f6bc6017",
    "work": "84c42ce600424c2d3c9b88b1e1a4b700c6b078c1b315ef6e51045504134489d9",
}
PROMPT = (
    "공개 이슈 #577을 수정하라. `saturn evidence search deployment settings`처럼 "
    "검색어를 따옴표 없이 여러 단어로 쓰면 `unexpected argument`가 나오고 engine에 조회가 "
    "가지 않는다. 다중 단어를 하나의 검색어로 받아 engine에 전달하도록 CLI를 수정하고, "
    "관련 회귀 검사를 추가해 실행하라. 작업 폴더 안의 코드만 바꿔라."
)
FEEDBACK = "방금 수정 뒤 검사에 실패했다. 원래 요청과 관련된 오류를 확인하고 한 번 더 수정하라."
OVERRIDES_COMMON = [
    ["context.safety_percent", "35"],
    ["provider.claude.context.window", "100000"],
    ["context.evidence.lookup", "true"],
    ["model.mode", "manual"],
    ["permission.mode", "full"],
]
ORDER = random.Random(7007).sample(["R", "J"], 2)
EXPECTED_CANDIDATE_BASIS_SHA = "2b9a0d4ce6a88bc34ad9cddfc929fb523ed4efc644e2d625c4cf443840b81337"
HIDDEN_TEST = """
    #[test]
    fn diagnostic_unquoted_evidence_search() {
        assert!(parse(&["evidence", "search", "deployment", "settings"]).is_ok());
    }
"""

sys.path.insert(0, str(ROOT / "docs/experiments/arm-execution-verifier/scripts"))
from engine_client import ATTACH_ENV, Client, Engine  # noqa: E402


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha(path: Path) -> str:
    return sha(path.read_bytes())


def tree_sha(root: Path) -> str:
    rows = []
    for item in sorted(root.rglob("*")):
        rel = item.relative_to(root).as_posix()
        if item.name == "engine.sock" or item.name.endswith(".lock") or "packet-capture" in item.parts:
            continue
        if item.is_symlink():
            rows.append([rel, "link", os.readlink(item)])
        elif item.is_file():
            rows.append([rel, "file", file_sha(item)])
    return sha((json.dumps(rows, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode())


def save(path: Path, value: object, secret: str = "") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if secret:
        value = redact_value(value, secret)
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, default=str) + "\n"
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w") as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    tmp.replace(path)


def redact_value(value: object, secret: str) -> object:
    if isinstance(value, str):
        return value.replace(secret, "[REDACTED]")
    if isinstance(value, bytes):
        return value.replace(secret.encode(), b"[REDACTED]")
    if isinstance(value, dict):
        return {redact_value(key, secret): redact_value(item, secret) for key, item in value.items()}
    if isinstance(value, list):
        return [redact_value(item, secret) for item in value]
    if isinstance(value, tuple):
        return tuple(redact_value(item, secret) for item in value)
    return value


def committed_protocol() -> None:
    files = [HERE / "design.md", HERE / "scripts/03-paired-diagnostic.py"]
    for path in files:
        rel = str(path.relative_to(ROOT))
        subprocess.run(["git", "ls-files", "--error-unmatch", rel], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", rel], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)


def source_check(binary: Path) -> None:
    expected_files = {"binary": binary, "raw": SOURCE / "source.raw.json", "db": SOURCE / "home/saturn.db"}
    for name, path in expected_files.items():
        if not path.is_file() or file_sha(path) != EXPECTED[name]:
            raise RuntimeError(f"{name} source seal mismatch")
    for name, path in (("home", SOURCE / "home"), ("work", SOURCE / "work")):
        if not path.is_dir() or tree_sha(path) != EXPECTED[name]:
            raise RuntimeError(f"{name} source tree mismatch")
    raw = json.loads((SOURCE / "source.raw.json").read_text())
    if raw.get("status") != "ok" or not raw.get("tasks") or any(x != "Done" for x in raw["tasks"].values()):
        raise RuntimeError("source task incomplete")
    if raw.get("work_sha256_before") != raw.get("work_sha256_after"):
        raise RuntimeError("source work changed")


def paths_and_key() -> tuple[Path, str]:
    binary_path = os.environ.get("ARM_ENGINE_BIN")
    key_path = os.environ.get("SATURN_DIAGNOSTIC_KEY_FILE")
    if not binary_path or not key_path:
        raise RuntimeError("ARM_ENGINE_BIN and SATURN_DIAGNOSTIC_KEY_FILE required")
    binary, key_file = Path(binary_path), Path(key_path)
    if not binary.is_file() or not key_file.is_file():
        raise RuntimeError("engine binary or protected key file missing")
    key = key_file.read_text().strip()
    if not key:
        raise RuntimeError("protected key file empty")
    return binary, key


def preflight() -> None:
    binary, _ = paths_and_key()
    committed_protocol()
    source_check(binary)
    candidate_basis = source_candidate_basis()
    if candidate_basis != {"count": 13, "sha256": EXPECTED_CANDIDATE_BASIS_SHA}:
        raise RuntimeError("source ToolResult candidate basis mismatch")
    args = (SOURCE / "work/saturn-terminal/cli/src/args.rs").read_text()
    if "mod tests" not in args or "fn parse(args: &[&str])" not in args or not args.rstrip().endswith("}"):
        raise RuntimeError("hidden test injection shape unavailable")
    db_rows(SOURCE / "home/saturn.db", 1)
    for arm in ORDER:
        if (RUN / f"t1{arm}").exists():
            raise RuntimeError("target attempt directory already exists; no automatic retry")
    print(json.dumps({"status": "ready", "source": "s1", "order": ORDER, "target_attempts": 0, "provider_calls": 0}))


def clone_source(arm: str) -> tuple[Path, Path, Path]:
    root = RUN / f"t1{arm}"
    if root.exists():
        raise RuntimeError("target attempt already exists; no automatic retry")
    root.mkdir(mode=0o700)
    home, work = root / "home", root / "work"
    shutil.copytree(SOURCE / "home", home, symlinks=True, ignore=shutil.ignore_patterns("engine.sock", "*.lock", "packet-capture"))
    shutil.copytree(SOURCE / "work", work, symlinks=True)
    if tree_sha(home) != EXPECTED["home"] or tree_sha(work) != EXPECTED["work"]:
        raise RuntimeError("target clone differs from source")
    save(root / "clone.json", {"source_home_tree_sha256": EXPECTED["home"], "source_work_tree_sha256": EXPECTED["work"]})
    return root, home, work


def db_rows(path: Path, chat: int) -> dict:
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    def rows(query: str) -> list[dict]:
        return [dict(row) for row in db.execute(query, (chat,))]
    out = {
        "packets": rows("SELECT * FROM handoff_packets WHERE chat_id=? ORDER BY id"),
        "items": rows("SELECT i.* FROM handoff_packet_items i JOIN handoff_packets p ON p.id=i.packet_id WHERE p.chat_id=? ORDER BY i.packet_id,i.rowid"),
        "usage": rows("SELECT * FROM usage WHERE chat_id=? ORDER BY id"),
        "judgments": rows("SELECT * FROM judgments WHERE chat_id=? ORDER BY id"),
        "events": rows("SELECT seq,body FROM events WHERE chat_id=? ORDER BY seq"),
        "sessions": rows("SELECT * FROM sessions WHERE chat_id=? ORDER BY id"),
        "runs": rows("SELECT id,chat_id,input_id,task_id,agent_id,session_id,provider,effect_scope,started_at,ended_at,end_kind,length(raw_gzip) AS raw_gzip_bytes,raw_hash,raw_size,changes_state,evidence_state,evidence_reason,evidence_events FROM runs WHERE chat_id=? ORDER BY id"),
        "evidence_lookups": rows("SELECT * FROM evidence_lookups WHERE chat_id=? ORDER BY id"),
    }
    db.close()
    return out


def source_candidate_basis() -> dict:
    db = sqlite3.connect(f"file:{SOURCE / 'home/saturn.db'}?mode=ro", uri=True)
    candidates = [(seq, sha(body.encode())) for seq, body in db.execute(
        "SELECT seq,body FROM events WHERE chat_id=1 AND body LIKE '%ToolResult%' ORDER BY seq"
    ) if body.startswith('{"ToolResult"')]
    db.close()
    return {"count": len(candidates), "sha256": sha((json.dumps(candidates, separators=(",", ":")) + "\n").encode())}


def file_contains_secret(path: Path, secret: bytes) -> bool:
    if not path.is_file() or path.is_symlink():
        return False
    overlap = b""
    with path.open("rb") as stream:
        while chunk := stream.read(1 << 20):
            data = overlap + chunk
            if secret in data:
                return True
            overlap = data[-(len(secret) - 1):] if len(secret) > 1 else b""
    return False


def value_contains_secret(value: object, secret: str) -> bool:
    if isinstance(value, str):
        return secret in value
    if isinstance(value, bytes):
        return secret.encode() in value
    if isinstance(value, dict):
        return any(value_contains_secret(key, secret) or value_contains_secret(item, secret) for key, item in value.items())
    if isinstance(value, (list, tuple)):
        return any(value_contains_secret(item, secret) for item in value)
    return False


def security_scan(root: Path, home: Path, work: Path, outputs: list[dict], secret: str) -> dict[str, bool]:
    needle = secret.encode()
    captures = home / "packet-capture"
    flags = {"raw": value_contains_secret(outputs, secret)}
    for name, paths in (
        ("db", (home / "saturn.db", home / "saturn.db-wal", home / "saturn.db-shm")),
        ("capture", captures.rglob("*") if captures.is_dir() else ()),
        ("work", work.rglob("*")),
        ("engine_log", (root / "engine.log",)),
    ):
        try:
            flags[name] = any(file_contains_secret(path, needle) for path in paths)
        except OSError:
            flags[name] = False
            flags["scan_error"] = True
    return flags


def hidden_check(work: Path, root: Path, suffix: str, secret: str) -> dict:
    args = work / "saturn-terminal/cli/src/args.rs"
    before = args.read_bytes()
    text = before.decode()
    if "mod tests" not in text or not text.rstrip().endswith("}"):
        return {"status": "injection_unavailable"}
    at = text.rfind("\n}")
    if at < 0:
        return {"status": "injection_unavailable"}
    args.write_text(text[:at] + HIDDEN_TEST + text[at:])
    try:
        result = subprocess.run(
            ["cargo", "test", "-p", "saturn-cli", "diagnostic_unquoted_evidence_search", "--offline"],
            cwd=work, capture_output=True, timeout=300,
            env={**{name: value for name, value in os.environ.items() if all(x not in name.upper() for x in ("KEY", "TOKEN", "SECRET", "PASSWORD"))},
                 "CARGO_TARGET_DIR": str(root / "cargo-target")},
        )
        output = result.stdout + b"\n" + result.stderr
        secret_detected = secret.encode() in output
        if secret_detected:
            output = output.replace(secret.encode(), b"[REDACTED]")
        path = root / f"hidden-{suffix}.log"
        path.write_bytes(output)
        return {"status": "complete", "exit_code": result.returncode, "log_sha256": file_sha(path), "secret_detected": secret_detected}
    except subprocess.TimeoutExpired as error:
        output = (error.stdout or b"") + b"\n" + (error.stderr or b"")
        path = root / f"hidden-{suffix}.log"
        secret_detected = secret.encode() in output
        path.write_bytes(output.replace(secret.encode(), b"[REDACTED]"))
        return {"status": "timeout", "log_sha256": file_sha(path), "secret_detected": secret_detected}
    finally:
        args.write_bytes(before)


def collect_one(arm: str, binary: Path, secret: str) -> None:
    root, home, work = clone_source(arm)
    overrides = [*OVERRIDES_COMMON, ["context.select.packet", "rrf" if arm == "R" else "jev"]]
    save(root / "plan.json", {"arm": arm, "order": ORDER, "prompt_sha256": sha(PROMPT.encode()),
                               "feedback_sha256": sha(FEEDBACK.encode()), "overrides": overrides,
                               "binary_sha256": EXPECTED["binary"], "model": "claude/sonnet", "source": "s1"})
    env = {name: value for name, value in os.environ.items() if all(x not in name.upper() for x in ("KEY", "TOKEN", "SECRET", "PASSWORD"))}
    env.update(SATURN_HOME=str(home), SATURN_KEY=secret, SATURN_PACKET_CAPTURE="1")
    engine = None
    client = None
    chat = 1
    outputs = []
    checks = []
    security_flags = {}
    started = time.monotonic()
    try:
        engine = Engine(binary, home, root / "engine.log", env=env)
        client = Client(engine.sock_path)
        attach_env = [[k, os.environ[k]] for k in ATTACH_ENV if k in os.environ and k != "PATH"]
        attach_env.append(["PATH", f"{binary.parent}:{os.environ['PATH']}"])
        client.call("Attach", {"chat": chat, "workdir": str(work), "env": attach_env, "overrides": overrides, "add_dirs": []})
        client.set_model(chat, "claude", "sonnet")
        save(root / "first.attempt.json", {"state": "submit_started", "at_ns": time.time_ns()})
        first = client.run_input(chat, PROMPT, allow=lambda _: False, timeout=900)
        outputs.append(first)
        save(root / "first.raw.json", first, secret)
        security_flags = security_scan(root, home, work, outputs, secret)
        complete = first.get("status") == "ok" and first.get("tasks") and all(x == "Done" for x in first["tasks"].values())
        if complete and not any(security_flags.values()):
            check = hidden_check(work, root, "first", secret)
            checks.append(check)
            security_flags = security_scan(root, home, work, outputs, secret)
            if check.get("secret_detected"):
                security_flags["hidden_check"] = True
            if not any(security_flags.values()) and check.get("status") == "complete" and check.get("exit_code") != 0:
                save(root / "feedback.attempt.json", {"state": "submit_started", "at_ns": time.time_ns()})
                feedback = client.run_input(chat, FEEDBACK, allow=lambda _: False, timeout=900)
                outputs.append(feedback)
                save(root / "feedback.raw.json", feedback, secret)
                security_flags = security_scan(root, home, work, outputs, secret)
                if not any(security_flags.values()) and feedback.get("status") == "ok" and feedback.get("tasks") and all(x == "Done" for x in feedback["tasks"].values()):
                    checks.append(hidden_check(work, root, "feedback", secret))
    finally:
        if client is not None:
            client.close()
        if engine is not None:
            engine.stop()
        final_security = security_scan(root, home, work, outputs, secret)
        final_security["hidden_check"] = any(check.get("secret_detected", False) for check in checks)
        security_flags = {name: bool(security_flags.get(name) or final_security.get(name)) for name in set(security_flags) | set(final_security)}
        save(root / "security.json", {"secret_matches": security_flags, "security_failure": any(security_flags.values())})
        log = root / "engine.log"
        if log.exists() and security_flags.get("engine_log"):
            log.write_bytes(log.read_bytes().replace(secret.encode(), b"[REDACTED]"))
        db_path = home / "saturn.db"
        data = db_rows(db_path, chat) if db_path.is_file() else {}
        captures = [json.loads(p.read_text()) for p in sorted((home / "packet-capture").glob("*.json"))] if (home / "packet-capture").exists() else []
        if value_contains_secret({"db": data, "captures": captures}, secret):
            security_flags["raw"] = True
            save(root / "security.json", {"secret_matches": security_flags, "security_failure": True})
        raw = {"arm": arm, "chat": chat, "overrides": overrides, "outputs": outputs, "checks": checks,
               "duration_s": time.monotonic() - started, "db": data, "captures": captures,
               "source_work_tree_sha256": EXPECTED["work"], "final_work_tree_sha256": tree_sha(work),
               "source_candidate_basis": source_candidate_basis(), "secret_matches": security_flags,
               "db_sha256": file_sha(db_path) if db_path.is_file() else None,
               "engine_log_sha256": file_sha(log) if log.exists() else None,
               "receipt": "unobserved"}
        save(root / "target.raw.json", raw, secret)
    if any(security_flags.values()):
        raise RuntimeError("protected key matched target output; stopped after preserving redacted raw")
    print(json.dumps({"arm": arm, "attempted": True, "outputs": len(outputs), "checks": len(checks),
                      "raw_sha256": file_sha(root / "target.raw.json"), "receipt": "unobserved"}))


def verify() -> None:
    rows = {}
    for arm in ("R", "J"):
        path = RUN / f"t1{arm}/target.raw.json"
        if not path.is_file():
            raise RuntimeError(f"target {arm} raw missing")
        raw = json.loads(path.read_text())
        packet = next((p for p in raw["db"]["packets"] if p["kind"] == "Switch" and p["state"] == "Sent"), None)
        if packet is None:
            rows[arm] = {"packet": "missing", "raw_sha256": file_sha(path)}
            continue
        capture = next((c for c in raw["captures"] if c.get("packet_id") == packet["id"]), None)
        body_match = bool(capture and sha(capture["body"].encode()) == packet["body_hash"] and len(capture["body"].encode()) == packet["body_bytes"])
        items = [i for i in raw["db"]["items"] if i["packet_id"] == packet["id"]]
        rows[arm] = {"packet": "found", "raw_sha256": file_sha(path), "requested_selector": packet["requested_selector"],
                     "actual_selector": packet["actual_selector"], "selection_fallback": packet["selection_fallback"],
                     "estimated_tokens": packet["estimated_tokens"], "body_capture_matches_db": body_match,
                     "protected": [(i["zone"], i["ref_id"], i["body_hash"]) for i in items if i["zone"] in ("User", "Assistant")],
                     "selected_competing": sorted((i["ref_id"], i["body_hash"], i["form"]) for i in items if i["zone"] == "Competing"),
                     "competing_items": sum(i["zone"] == "Competing" for i in items), "receipt": "unobserved"}
    same_protected = rows["R"].get("protected") == rows["J"].get("protected") if all(x.get("packet") == "found" for x in rows.values()) else None
    raw_r = json.loads((RUN / "t1R/target.raw.json").read_text())
    raw_j = json.loads((RUN / "t1J/target.raw.json").read_text())
    same_candidates = raw_r["source_candidate_basis"] == raw_j["source_candidate_basis"] == source_candidate_basis()
    same_budget = all(
        ["context.safety_percent", "35"] in raw["overrides"]
        and ["provider.claude.context.window", "100000"] in raw["overrides"]
        and ["context.evidence.lookup", "true"] in raw["overrides"]
        for raw in (raw_r, raw_j)
    )
    same_source = all(
        raw["source_work_tree_sha256"] == EXPECTED["work"]
        and json.loads((RUN / f"t1{arm}/clone.json").read_text())["source_home_tree_sha256"] == EXPECTED["home"]
        for arm, raw in (("R", raw_r), ("J", raw_j))
    )
    save(RUN / "paired-verification.json", {"arms": rows, "same_protected": same_protected,
                                             "same_candidate_basis": same_candidates,
                                             "same_source_home_and_work": same_source, "same_budget_settings": same_budget,
                                             "calculated_packet_budget_equal": None,
                                             "calculated_packet_budget_note": "runtime budget not independently observed",
                                             "provider_receipt": "unobserved", "confirmatory_effect": "not_evaluated"})
    print(json.dumps({"packet_R": rows["R"]["packet"], "packet_J": rows["J"]["packet"],
                      "same_protected": same_protected, "same_candidate_basis": same_candidates,
                      "provider_receipt": "unobserved"}))


def main() -> None:
    os.umask(0o077)
    if len(sys.argv) != 2 or sys.argv[1] not in ("preflight", "collect", "verify"):
        raise SystemExit("usage: 03-paired-diagnostic.py preflight|collect|verify")
    if sys.argv[1] == "verify":
        verify()
        return
    if sys.argv[1] == "preflight":
        preflight()
        return
    preflight()
    binary, key = paths_and_key()
    for arm in ORDER:
        collect_one(arm, binary, key)
    verify()


if __name__ == "__main__":
    main()
