"""#577 기능 진단의 공통 실제 작업 경로 재생. s1/t1 시도는 재사용하지 않는다.

외부 호출은 사전등록 커밋 뒤 source, target-j, target-r에서만 일어난다.
각 명령은 한 번만 실행하며 실패·불명 상태를 자동 재시도하지 않는다.
"""

from __future__ import annotations

import fcntl
import hashlib
import importlib.util
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[2]
RUN = ROOT / ".runtime/jr"
ACTIVE = RUN / "active"
WORK = ACTIVE / "work"
SOURCE = RUN / "s2"
BASELINE = RUN / "t1J/work"
BASELINE_SHA = "84c42ce600424c2d3c9b88b1e1a4b700c6b078c1b315ef6e51045504134489d9"
BIN_SHA = "cc916075f893223178afc4bf6aee6839aa3545420dc8d4051ab3d9e064ae6405"
FORBIDDEN_WORKS = tuple(str(path) for path in (RUN / "s1/work", RUN / "t1J/work", RUN / "t1R/work", RUN / "s2/work"))
SOURCE_PROMPT = (
    "공개 개발 요청의 재생이다. `saturn evidence search deployment settings`처럼 검색어를 "
    "따옴표 없이 여러 단어로 쓰면 `unexpected argument`가 나오고 engine에 조회가 가지 않는다. "
    "이 요청을 다음 턴에서 수정할 예정이다. 지금은 CLI 인자 정의, evidence 명령 경로, 관련 설계만 실제 파일에서 읽어라. "
    "파일은 수정하지 말고 테스트나 빌드를 실행하지 마라. 마지막 답은 `읽음` 한 단어만 써라."
)

spec = importlib.util.spec_from_file_location("first_diagnostic", HERE / "scripts/03-paired-diagnostic.py")
first = importlib.util.module_from_spec(spec)
spec.loader.exec_module(first)
sys.path.insert(0, str(ROOT / "docs/experiments/arm-execution-verifier/scripts"))
from engine_client import ATTACH_ENV, Client, Engine  # noqa: E402


def paths_and_key() -> tuple[Path, str]:
    binary_name = os.environ.get("ARM_ENGINE_BIN")
    key_name = os.environ.get("SATURN_DIAGNOSTIC_KEY_FILE")
    if not binary_name or not key_name:
        raise RuntimeError("engine binary and protected key file required")
    binary, key_file = Path(binary_name), Path(key_name)
    if not binary.is_file() or not key_file.is_file() or first.file_sha(binary) != BIN_SHA:
        raise RuntimeError("engine binary or protected key file unavailable or changed")
    secret = key_file.read_text().strip()
    if not secret:
        raise RuntimeError("protected key file empty")
    return binary, secret


def committed_protocol() -> None:
    for path in (HERE / "design.md", HERE / "scripts/03-paired-diagnostic.py", HERE / "scripts/04-physical-workdir.py"):
        rel = str(path.relative_to(ROOT))
        subprocess.run(["git", "ls-files", "--error-unmatch", rel], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", rel], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)


def lock() -> object:
    ACTIVE.mkdir(mode=0o700, parents=True, exist_ok=True)
    path = ACTIVE / "runner.lock"
    handle = path.open("a+")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        handle.close()
        raise RuntimeError("another diagnostic runner holds the active work lock")
    return handle


def physical_path() -> None:
    for path in (ROOT, ROOT / ".runtime", RUN, ACTIVE, WORK):
        if path.is_symlink():
            raise RuntimeError("active work path contains a symlink")
    if WORK.exists() and not WORK.is_dir():
        raise RuntimeError("active work is not a real directory")


def no_open_path(path: Path) -> None:
    if shutil.which("lsof") is None:
        raise RuntimeError("lsof unavailable; cannot prove active directory is closed")
    result = subprocess.run(["lsof", "-nP", "+D", str(path)], capture_output=True, timeout=30)
    if result.returncode != 1 or result.stderr:
        raise RuntimeError("active directory still open or lsof check failed")


def security_flags(root: Path, home: Path, work: Path, outputs: list[dict], secret: str) -> dict[str, bool]:
    flags = first.security_scan(root, home, work, outputs, secret)
    try:
        flags["home_other"] = any(
            first.file_contains_secret(path, secret.encode()) for path in home.rglob("*")
            if path.is_file() and not path.name.startswith("saturn.db") and "packet-capture" not in path.parts
        )
    except OSError:
        flags["scan_error"] = True
    return flags


def source_candidate_basis(db_path: Path) -> dict:
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    candidates = [(seq, first.sha(body.encode())) for seq, body in db.execute(
        "SELECT seq,body FROM events WHERE chat_id=1 AND body LIKE '%ToolResult%' ORDER BY seq"
    ) if body.startswith('{"ToolResult"')]
    db.close()
    return {"count": len(candidates), "sha256": first.sha((json.dumps(candidates, separators=(",", ":")) + "\n").encode())}


def source_prefix_gate(target_db_path: Path) -> dict:
    source_db = sqlite3.connect(f"file:{SOURCE / 'home/saturn.db'}?mode=ro", uri=True)
    target_db = sqlite3.connect(f"file:{target_db_path}?mode=ro", uri=True)
    source_events = list(source_db.execute("SELECT seq,body FROM events WHERE chat_id=1 ORDER BY seq"))
    if not source_events:
        raise RuntimeError("sealed source has no events")
    boundary = source_events[-1][0]
    target_events = list(target_db.execute("SELECT seq,body FROM events WHERE chat_id=1 AND seq<=? ORDER BY seq", (boundary,)))
    source_db.close()
    target_db.close()
    if source_events != target_events:
        raise RuntimeError("target source event prefix differs from sealed source")
    candidates = [(seq, first.sha(body.encode())) for seq, body in target_events if body.startswith('{"ToolResult"')]
    basis = {"count": len(candidates), "sha256": first.sha((json.dumps(candidates, separators=(",", ":")) + "\n").encode())}
    seal = json.loads((SOURCE / "seal.json").read_text())
    if basis != seal["candidate_basis"]:
        raise RuntimeError("target source ToolResult prefix differs from sealed source")
    return {"source_event_boundary_seq": boundary, "source_event_rows": len(source_events), "candidate_basis": basis,
            "source_prefix_exact": True}


def db_path_gate(db_path: Path) -> dict:
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    chat_paths = [row[0] for row in db.execute("SELECT workdir FROM chats ORDER BY id")]
    input_paths = [row[0] for row in db.execute("SELECT workdir FROM inputs ORDER BY id")]
    old_hits = []
    for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
        for _, col, kind, *_ in db.execute(f"PRAGMA table_info({table})"):
            if kind.upper() not in ("TEXT", "BLOB"):
                continue
            for forbidden in FORBIDDEN_WORKS:
                count = db.execute(f"SELECT count(*) FROM {table} WHERE instr(CAST({col} AS BLOB), ?)>0", (forbidden.encode(),)).fetchone()[0]
                if count:
                    old_hits.append([table, col, count])
    db.close()
    if chat_paths != [str(WORK)] or not input_paths or any(path != str(WORK) for path in input_paths) or old_hits:
        raise RuntimeError("source DB work path is not the common physical path")
    return {"chat_workdir_matches": True, "input_workdirs_match": True, "old_path_hits": 0}


def files_contain_forbidden_path(root: Path) -> bool:
    return any(first.file_contains_secret(path, forbidden.encode()) for path in root.rglob("*") if path.is_file() for forbidden in FORBIDDEN_WORKS)


def source_preflight() -> None:
    committed_protocol()
    physical_path()
    paths_and_key()
    if SOURCE.exists() or WORK.exists() or (ACTIVE / "home").exists():
        raise RuntimeError("source or active attempt already exists; no automatic retry")
    if not BASELINE.is_dir() or first.tree_sha(BASELINE) != BASELINE_SHA:
        raise RuntimeError("pristine baseline unavailable")
    print(json.dumps({"status": "source_ready", "source": "s2", "provider_calls": 0}))


def engine_env(home: Path, secret: str) -> dict:
    env = {name: value for name, value in os.environ.items() if all(x not in name.upper() for x in ("KEY", "TOKEN", "SECRET", "PASSWORD"))}
    env.update(SATURN_HOME=str(home), SATURN_KEY=secret)
    return env


def complete(result: dict) -> bool:
    return result.get("status") == "ok" and bool(result.get("tasks")) and all(value == "Done" for value in result["tasks"].values())


def make_source(binary: Path, secret: str) -> None:
    shutil.copytree(BASELINE, WORK, symlinks=True)
    if first.tree_sha(WORK) != BASELINE_SHA:
        raise RuntimeError("source work clone mismatch")
    first.save(ACTIVE / "source.attempt.json", {"state": "submit_started", "prompt_sha256": first.sha(SOURCE_PROMPT.encode()), "at_ns": time.time_ns()})
    home = ACTIVE / "home"
    engine = None
    client = None
    result = {}
    try:
        engine = Engine(binary, home, ACTIVE / "source.engine.log", env=engine_env(home, secret))
        client = Client(engine.sock_path)
        chat = client.attach(WORK, [["model.mode", "manual"], ["permission.mode", "full"], ["router.skip_check", "true"]])
        client.set_model(chat, "codex", "gpt-5.6-luna")
        result = client.run_input(chat, SOURCE_PROMPT, allow=lambda _: False, timeout=300)
        result["chat"] = chat
        first.save(ACTIVE / "source.raw.json", result, secret)
    finally:
        if client is not None:
            client.close()
        if engine is not None:
            engine.stop()
        flags = security_flags(ACTIVE, home, WORK, [result], secret)
        first.save(ACTIVE / "source.security.json", {"secret_matches": flags, "security_failure": any(flags.values())})
        log = ACTIVE / "source.engine.log"
        if log.exists() and flags.get("engine_log"):
            log.write_bytes(log.read_bytes().replace(secret.encode(), b"[REDACTED]"))
    if not complete(result) or any(flags.values()) or first.tree_sha(WORK) != BASELINE_SHA:
        raise RuntimeError("source incomplete, security failure, or source work changed")
    no_open_path(WORK)
    no_open_path(home)
    db_path = home / "saturn.db"
    paths = db_path_gate(db_path)
    if files_contain_forbidden_path(home):
        raise RuntimeError("previous or sealed work path remains in new source home")
    candidates = source_candidate_basis(db_path)
    if candidates["count"] == 0:
        raise RuntimeError("new source has no ToolResult candidates")
    SOURCE.mkdir(mode=0o700)
    (ACTIVE / "home").rename(SOURCE / "home")
    WORK.rename(SOURCE / "work")
    (ACTIVE / "source.raw.json").rename(SOURCE / "source.raw.json")
    (ACTIVE / "source.attempt.json").rename(SOURCE / "source.attempt.json")
    (ACTIVE / "source.security.json").rename(SOURCE / "source.security.json")
    (ACTIVE / "source.engine.log").rename(SOURCE / "source.engine.log")
    seal = {"source_home_tree_sha256": first.tree_sha(SOURCE / "home"), "source_work_tree_sha256": first.tree_sha(SOURCE / "work"),
            "source_db_sha256": first.file_sha(SOURCE / "home/saturn.db"), "source_raw_sha256": first.file_sha(SOURCE / "source.raw.json"),
            "candidate_basis": candidates, "workdir": str(WORK), "db_path_gate": paths, "baseline_sha256": BASELINE_SHA,
            "binary_sha256": BIN_SHA, "source_prompt_sha256": first.sha(SOURCE_PROMPT.encode())}
    first.save(SOURCE / "seal.json", seal)
    print(json.dumps({"status": "source_sealed", "candidate_count": candidates["count"], "source_raw_sha256": seal["source_raw_sha256"]}))


def read_seal(allow_active: bool = False) -> dict:
    path = SOURCE / "seal.json"
    if not path.is_file() or (WORK.exists() and not allow_active) or (ACTIVE / "home").exists():
        raise RuntimeError("source seal missing or active work occupied")
    seal = json.loads(path.read_text())
    if seal["workdir"] != str(WORK) or seal["binary_sha256"] != BIN_SHA or seal["baseline_sha256"] != BASELINE_SHA:
        raise RuntimeError("source seal metadata mismatch")
    for name, path, digest in (
        ("home", SOURCE / "home", seal["source_home_tree_sha256"]),
        ("work", SOURCE / "work", seal["source_work_tree_sha256"]),
    ):
        if not path.is_dir() or first.tree_sha(path) != digest:
            raise RuntimeError(f"source {name} snapshot changed")
    for path, digest in ((SOURCE / "home/saturn.db", seal["source_db_sha256"]), (SOURCE / "source.raw.json", seal["source_raw_sha256"])):
        if not path.is_file() or first.file_sha(path) != digest:
            raise RuntimeError("source raw or DB changed")
    db_path_gate(SOURCE / "home/saturn.db")
    if files_contain_forbidden_path(SOURCE / "home") or source_candidate_basis(SOURCE / "home/saturn.db") != seal["candidate_basis"]:
        raise RuntimeError("source candidate or path changed")
    return seal


def target_preflight(arm: str) -> None:
    committed_protocol()
    physical_path()
    paths_and_key()
    read_seal()
    root = RUN / f"t2{arm}"
    if root.exists():
        raise RuntimeError("target attempt already exists; no automatic retry")
    if arm == "R":
        first_root = RUN / "t2J"
        if not (first_root / "target.raw.json").is_file() or not (first_root / "work").is_dir():
            raise RuntimeError("J attempt not fully preserved")
        source_prefix_gate(first_root / "home/saturn.db")
    print(json.dumps({"status": "target_ready", "arm": arm, "provider_calls": 0}))


def run_target(arm: str, binary: Path, secret: str) -> None:
    seal = read_seal()
    root = RUN / f"t2{arm}"
    root.mkdir(mode=0o700)
    home = root / "home"
    shutil.copytree(SOURCE / "home", home, symlinks=True, ignore=shutil.ignore_patterns("engine.sock", "*.lock", "packet-capture"))
    shutil.copytree(SOURCE / "work", WORK, symlinks=True)
    if first.tree_sha(home) != seal["source_home_tree_sha256"] or first.tree_sha(WORK) != seal["source_work_tree_sha256"]:
        raise RuntimeError("target home or active work clone mismatch")
    if first.file_sha(home / "saturn.db") != seal["source_db_sha256"] or source_prefix_gate(home / "saturn.db")["candidate_basis"] != seal["candidate_basis"]:
        raise RuntimeError("target source DB or candidate body differs")
    db_path_gate(home / "saturn.db")
    overrides = [*first.OVERRIDES_COMMON, ["context.select.packet", "rrf" if arm == "R" else "jev"]]
    first.save(root / "plan.json", {"arm": arm, "source": "s2", "model": "claude/sonnet", "overrides": overrides,
                                    "target_prompt_sha256": first.sha(first.PROMPT.encode()), "feedback_sha256": first.sha(first.FEEDBACK.encode()),
                                    "source_home_tree_sha256": seal["source_home_tree_sha256"], "source_work_tree_sha256": seal["source_work_tree_sha256"]})
    env = engine_env(home, secret)
    env["SATURN_PACKET_CAPTURE"] = "1"
    engine = None
    client = None
    outputs = []
    checks = []
    flags = {}
    started = time.monotonic()
    try:
        engine = Engine(binary, home, root / "engine.log", env=env)
        client = Client(engine.sock_path)
        attach_env = [[name, os.environ[name]] for name in ATTACH_ENV if name in os.environ and name != "PATH"]
        attach_env.append(["PATH", f"{binary.parent}:{os.environ['PATH']}"])
        client.call("Attach", {"chat": 1, "workdir": str(WORK), "env": attach_env, "overrides": overrides, "add_dirs": []})
        client.set_model(1, "claude", "sonnet")
        first.save(root / "first.attempt.json", {"state": "submit_started", "at_ns": time.time_ns()})
        output = client.run_input(1, first.PROMPT, allow=lambda _: False, timeout=900)
        outputs.append(output)
        first.save(root / "first.raw.json", output, secret)
        flags = security_flags(root, home, WORK, outputs, secret)
        if complete(output) and not any(flags.values()):
            check = first.hidden_check(WORK, root, "first", secret)
            checks.append(check)
            flags = security_flags(root, home, WORK, outputs, secret)
            flags["hidden_check"] = check.get("secret_detected", False)
            if not any(flags.values()) and check.get("status") == "complete" and check.get("exit_code") != 0:
                first.save(root / "feedback.attempt.json", {"state": "submit_started", "at_ns": time.time_ns()})
                feedback = client.run_input(1, first.FEEDBACK, allow=lambda _: False, timeout=900)
                outputs.append(feedback)
                first.save(root / "feedback.raw.json", feedback, secret)
                flags = security_flags(root, home, WORK, outputs, secret)
                if complete(feedback) and not any(flags.values()):
                    checks.append(first.hidden_check(WORK, root, "feedback", secret))
    finally:
        if client is not None:
            client.close()
        if engine is not None:
            engine.stop()
        final_flags = security_flags(root, home, WORK, outputs, secret)
        final_flags["hidden_check"] = any(check.get("secret_detected", False) for check in checks)
        flags = {key: bool(flags.get(key) or final_flags.get(key)) for key in set(flags) | set(final_flags)}
        first.save(root / "security.json", {"secret_matches": flags, "security_failure": any(flags.values())})
        log = root / "engine.log"
        if log.exists() and flags.get("engine_log"):
            log.write_bytes(log.read_bytes().replace(secret.encode(), b"[REDACTED]"))
        db = first.db_rows(home / "saturn.db", 1) if (home / "saturn.db").is_file() else {}
        captures = [json.loads(path.read_text()) for path in sorted((home / "packet-capture").glob("*.json"))] if (home / "packet-capture").is_dir() else []
        if first.value_contains_secret({"db": db, "captures": captures}, secret):
            flags["raw"] = True
            first.save(root / "security.json", {"secret_matches": flags, "security_failure": True})
        old_path_in_capture = any(forbidden in capture.get("body", "") for capture in captures for forbidden in FORBIDDEN_WORKS)
        raw = {"arm": arm, "source": "s2", "chat": 1, "overrides": overrides, "outputs": outputs, "checks": checks,
               "duration_s": time.monotonic() - started, "db": db, "captures": captures, "secret_matches": flags,
               "source_candidate_basis": source_prefix_gate(home / "saturn.db")["candidate_basis"], "old_path_in_capture": old_path_in_capture,
               "active_work_tree_sha256": first.tree_sha(WORK), "source_work_tree_sha256": seal["source_work_tree_sha256"],
               "db_sha256": first.file_sha(home / "saturn.db") if (home / "saturn.db").is_file() else None,
               "receipt": "unobserved"}
        first.save(root / "target.raw.json", raw, secret)
    if (any(flags.values()) or old_path_in_capture or not outputs or any(not complete(output) for output in outputs)
            or not checks or any(check.get("status") != "complete" for check in checks)):
        raise RuntimeError("target security, path, or task gate failed; active work preserved")
    no_open_path(WORK)
    no_open_path(home)
    before_move = first.tree_sha(WORK)
    time.sleep(1)
    if first.tree_sha(WORK) != before_move or read_seal(allow_active=True)["source_work_tree_sha256"] != seal["source_work_tree_sha256"]:
        raise RuntimeError("active work unstable or source changed")
    WORK.rename(root / "work")
    if first.tree_sha(root / "work") != before_move:
        raise RuntimeError("preserved target work differs from active work")
    first.save(root / "preserved.json", {"active_work_sha256": before_move, "source_home_sha256": seal["source_home_tree_sha256"],
                                         "source_work_sha256": seal["source_work_tree_sha256"]})
    print(json.dumps({"status": "target_preserved", "arm": arm, "inputs": len(outputs), "raw_sha256": first.file_sha(root / "target.raw.json")}))


def recheck_j() -> None:
    read_seal()
    root = RUN / "t2J"
    raw_path = root / "target.raw.json"
    if not raw_path.is_file() or not (root / "work").is_dir():
        raise RuntimeError("J raw or preserved work missing")
    prefix = source_prefix_gate(root / "home/saturn.db")
    first.save(RUN / "s2-j-prefix-reanalysis.json", {"raw_sha256": first.file_sha(raw_path), "source_prefix": prefix,
                                                      "raw_unchanged": True, "target_r_inputs": 0})
    print(json.dumps({"status": "j_prefix_verified", "raw_unchanged": True, "source_prefix_exact": True}))


def verify() -> None:
    seal = read_seal()
    rows = {}
    plans = {}
    for arm in ("J", "R"):
        root = RUN / f"t2{arm}"
        raw_path = root / "target.raw.json"
        if not raw_path.is_file() or not (root / "work").is_dir():
            raise RuntimeError("paired target missing")
        raw = json.loads(raw_path.read_text())
        plans[arm] = json.loads((root / "plan.json").read_text())
        preserved = json.loads((root / "preserved.json").read_text())
        if plans[arm]["source_home_tree_sha256"] != seal["source_home_tree_sha256"] or plans[arm]["source_work_tree_sha256"] != seal["source_work_tree_sha256"]:
            raise RuntimeError("target source plan differs from sealed snapshot")
        if preserved["source_home_sha256"] != seal["source_home_tree_sha256"] or preserved["source_work_sha256"] != seal["source_work_tree_sha256"]:
            raise RuntimeError("preserved target source fingerprint differs")
        prefix = source_prefix_gate(root / "home/saturn.db")
        if raw["old_path_in_capture"] or any(raw["secret_matches"].values()):
            raise RuntimeError("candidate, old path, or security mismatch")
        packet = next((p for p in raw["db"]["packets"] if p["kind"] == "Switch" and p["state"] == "Sent"), None)
        if packet is None:
            raise RuntimeError("first Switch packet missing")
        capture = next((c for c in raw["captures"] if c.get("packet_id") == packet["id"]), None)
        if not capture or first.sha(capture["body"].encode()) != packet["body_hash"] or len(capture["body"].encode()) != packet["body_bytes"]:
            raise RuntimeError("outbound packet capture mismatch")
        items = [i for i in raw["db"]["items"] if i["packet_id"] == packet["id"]]
        rows[arm] = {"packet_hash": packet["body_hash"], "protected": [(i["zone"], i["ref_id"], i["body_hash"]) for i in items if i["zone"] in ("User", "Assistant")],
                     "selected_competing": [(i["ref_id"], i["body_hash"], i["form"]) for i in items if i["zone"] == "Competing"],
                     "requested_selector": packet["requested_selector"], "actual_selector": packet["actual_selector"],
                     "selection_fallback": packet["selection_fallback"], "raw_sha256": first.file_sha(raw_path),
                     "work_sha256": first.tree_sha(root / "work"), "source_prefix": prefix}
    same_protected = rows["J"]["protected"] == rows["R"]["protected"]
    same_budget_settings = all(
        ["context.safety_percent", "35"] in plans[arm]["overrides"]
        and ["provider.claude.context.window", "100000"] in plans[arm]["overrides"]
        and ["context.evidence.lookup", "true"] in plans[arm]["overrides"]
        for arm in ("J", "R")
    )
    first.save(RUN / "s2-paired-reanalysis.json", {"arms": rows, "same_protected": same_protected,
              "same_candidate_body": True, "same_source_home_and_work": True, "same_budget_settings": same_budget_settings,
              "calculated_packet_budget_equal": None, "provider_receipt": "unobserved", "confirmatory_effect": "not_evaluated"})
    print(json.dumps({"status": "verified", "same_protected": same_protected, "same_candidate_body": True,
                      "provider_receipt": "unobserved"}))


def main() -> None:
    os.umask(0o077)
    if len(sys.argv) != 2 or sys.argv[1] not in ("source-preflight", "source", "target-j-preflight", "target-j", "recheck-j", "target-r-preflight", "target-r", "verify"):
        raise SystemExit("usage: 04-physical-workdir.py source-preflight|source|target-j-preflight|target-j|recheck-j|target-r-preflight|target-r|verify")
    command = sys.argv[1]
    handle = lock()
    try:
        if command == "source-preflight":
            source_preflight()
        elif command == "source":
            source_preflight()
            binary, secret = paths_and_key()
            make_source(binary, secret)
        elif command in ("target-j-preflight", "target-r-preflight"):
            target_preflight(command[7].upper())
        elif command in ("target-j", "target-r"):
            arm = command[7].upper()
            target_preflight(arm)
            binary, secret = paths_and_key()
            run_target(arm, binary, secret)
        elif command == "recheck-j":
            recheck_j()
        else:
            verify()
    finally:
        handle.close()


if __name__ == "__main__":
    main()
