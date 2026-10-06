"""#577 공개 실과제의 새 source 대화를 한 번 생성하는 예비 진단.

확인 평가 80쌍과 별개다. 미커밋 상태에서 만든 source는 효과 판정에 쓰지 않는다.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[2]
RUN = ROOT / ".runtime/jr"
EXPECTED_BIN_SHA = "cc916075f893223178afc4bf6aee6839aa3545420dc8d4051ab3d9e064ae6405"
BASE_COMMIT = "7f533049a32233332e70020aff05f2bbf98702ad"
EXPECTED_WORK_SHA = "375a79a9ded6cece3178974a8f0c408aac8b1589173c40d1020244da7f2aa384"
PROMPT = (
    "공개 개발 요청의 재생이다. `saturn evidence search deployment settings`처럼 검색어를 "
    "따옴표 없이 여러 단어로 쓰면 `unexpected argument`가 나오고 engine에 조회가 가지 않는다. "
    "이 요청을 다음 턴에서 수정할 예정이다. 지금은 CLI 인자 정의, evidence 명령 경로, 관련 설계만 실제 파일에서 읽어라. "
    "파일은 수정하지 말고 테스트나 빌드를 실행하지 마라. 마지막 답은 `읽음` 한 단어만 써라."
)
sys.path.insert(0, str(ROOT / "docs/experiments/arm-execution-verifier/scripts"))
from engine_client import Client, Engine  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tree_digest(root: Path) -> str:
    rows = []
    for path in sorted(root.rglob("*")):
        if path.is_file() and not path.is_symlink():
            rows.append((path.relative_to(root).as_posix(), digest(path.read_bytes())))
    return digest(json.dumps(rows, separators=(",", ":")).encode())


def main() -> None:
    os.umask(0o077)
    attempt = os.environ.get("SATURN_DIAGNOSTIC_ATTEMPT", "")
    if len(attempt) != 2 or attempt[0] != "s" or not attempt[1].isdigit():
        raise SystemExit("set SATURN_DIAGNOSTIC_ATTEMPT to a new source id such as s1")
    home = RUN / attempt
    work = home / "work"
    engine_home = home / "home"
    binary_name = os.environ.get("ARM_ENGINE_BIN")
    key_file_name = os.environ.get("SATURN_DIAGNOSTIC_KEY_FILE")
    if not binary_name or not key_file_name:
        raise SystemExit("engine binary and protected key-file paths are required")
    binary = Path(binary_name)
    key_file = Path(key_file_name)
    if not binary.is_file() or not key_file.is_file():
        raise SystemExit("engine binary or protected key file missing")
    router_key = key_file.read_text().strip()
    if not router_key:
        raise SystemExit("protected key file is empty")
    if not work.is_dir() or (home / "source.raw.json").exists() or engine_home.exists():
        raise SystemExit("source work missing or source already attempted; no automatic retry")
    if digest(binary.read_bytes()) != EXPECTED_BIN_SHA:
        raise SystemExit("engine binary hash changed")
    if (work / ".git").exists():
        raise SystemExit("source work contains Git history")
    before = tree_digest(work)
    if before != EXPECTED_WORK_SHA:
        raise SystemExit("source work differs from fixed pre-fix commit")
    (home / "attempt.json").write_text(json.dumps({"state": "started", "at": datetime.now(timezone.utc).isoformat(), "base_commit": BASE_COMMIT, "prompt_sha256": digest(PROMPT.encode()), "work_sha256": before}, ensure_ascii=False) + "\n")
    env = {name: value for name, value in os.environ.items() if all(term not in name.upper() for term in ("KEY", "TOKEN", "SECRET", "PASSWORD"))}
    env["SATURN_HOME"] = str(engine_home)
    env["SATURN_KEY"] = router_key
    engine = Engine(binary, engine_home, home / "engine.log", env=env)
    client = Client(engine.sock_path)
    result = {}
    try:
        chat = client.attach(work, [["model.mode", "manual"], ["permission.mode", "full"], ["router.skip_check", "true"]])
        client.set_model(chat, "codex", "gpt-5.6-luna")
        result = client.run_input(chat, PROMPT, allow=lambda _: False, timeout=300)
        result["chat"] = chat
    finally:
        client.close()
        engine.stop()
        result["work_sha256_before"] = before
        result["work_sha256_after"] = tree_digest(work)
        result["prompt_sha256"] = digest(PROMPT.encode())
        result["base_commit"] = BASE_COMMIT
        raw = json.dumps(result, ensure_ascii=False, sort_keys=True, default=str) + "\n"
        result["secret_detected_in_result"] = router_key in raw
        log_file = home / "engine.log"
        if log_file.exists():
            log = log_file.read_bytes()
            if router_key.encode() in log:
                log_file.write_bytes(log.replace(router_key.encode(), b"[REDACTED]"))
                result["secret_detected_in_log"] = True
        raw = json.dumps(result, ensure_ascii=False, sort_keys=True, default=str) + "\n"
        (home / "source.raw.json").write_text(raw.replace(router_key, "[REDACTED]"))
    tasks = result.get("tasks") or {}
    print(json.dumps({"status": result.get("status"), "tasks": tasks, "chat": result.get("chat"), "work_unchanged": before == result["work_sha256_after"]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
