#!/usr/bin/env python3
"""provider-live-settings 수집 진입점. 수집 시점에 inventory와 trial을 기록한다."""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKTREE = ROOT.parents[2]
MAIN_REPO = Path.home() / "workspace" / "oss" / "saturn"
RAW = ROOT / "data" / "raw"
PRIVATE = MAIN_REPO / ".local" / "experiments" / "provider-live-settings"


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def redact(text: str) -> str:
    value = text.replace(str(Path.home()), "~")
    for name in ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "SATURN_JUDGE_KEY"):
        secret = os.environ.get(name)
        if secret:
            value = value.replace(secret, "[secret]")
    value = re.sub(r"(?i)(bearer\s+)[A-Za-z0-9._=-]+", r"\1[secret]", value)
    value = re.sub(r"(?i)(api[_-]?key|token|secret)\s*[:=]\s*[^,\s}\]]+", r"\1=[secret]", value)
    return value


def run(command: list[str], *, timeout: int = 30) -> tuple[int, str, str]:
    try:
        result = subprocess.run(command, cwd=WORKTREE, capture_output=True, text=True,
                                timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        return 127, "", redact(str(error))
    return result.returncode, redact(result.stdout), redact(result.stderr)


def save_private(name: str, content: str) -> str:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    path = PRIVATE / name
    path.write_text(redact(content), encoding="utf-8")
    return str(path.relative_to(MAIN_REPO))


def version(command: str) -> dict:
    code, stdout, stderr = run([command, "--version"])
    return {"command": command, "exit_code": code, "stdout": stdout, "stderr": stderr}


def inventory(run_id: str) -> list[dict]:
    rows: list[dict] = []
    for command in ("claude", "codex"):
        rows.append({"run_id": run_id, "trial_id": f"inventory-{command}-version",
                     "condition": f"inventory.{command}.version", "ts_utc": now(),
                     "provider": command, "request_result": "observed",
                     "process_id": "none", "tool_decision": "not_requested",
                     "fixture_effect": False, "next_turn_effect": False,
                     "restart_effect": False, "private_log": save_private(
                         f"inventory-{command}-version.txt", json.dumps(version(command), ensure_ascii=False))})
        code, stdout, stderr = run([command, "--help"], timeout=60)
        log = f"exit={code}\nstdout=\n{stdout}\nstderr=\n{stderr}\n"
        rows.append({"run_id": run_id, "trial_id": f"inventory-{command}-help",
                     "condition": f"inventory.{command}.help", "ts_utc": now(),
                     "provider": command, "request_result": "observed" if code == 0 else "error",
                     "process_id": "none", "tool_decision": "not_requested",
                     "fixture_effect": False, "next_turn_effect": False,
                     "restart_effect": False, "private_log": save_private(
                         f"inventory-{command}-help.txt", log)})
    code, stdout, stderr = run(["codex", "app-server", "generate-json-schema"], timeout=60)
    schema_log = f"exit={code}\nstdout=\n{stdout}\nstderr=\n{stderr}\n"
    schema_path = WORKTREE / ".runtime" / "codex-app-server-schema.json"
    schema_path.parent.mkdir(parents=True, exist_ok=True)
    schema_path.write_text(stdout, encoding="utf-8")
    rows.append({"run_id": run_id, "trial_id": "inventory-codex-schema",
                 "condition": "inventory.codex.app-server-schema", "ts_utc": now(),
                 "provider": "codex", "request_result": "observed" if code == 0 else "error",
                 "process_id": "none", "tool_decision": "not_requested",
                 "fixture_effect": False, "next_turn_effect": False,
                 "restart_effect": False, "private_log": save_private(
                     "inventory-codex-app-server-schema.txt", schema_log),
                 "schema_sha256": hashlib.sha256(stdout.encode()).hexdigest()})
    return rows


def main() -> int:
    commit = subprocess.check_output(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE, text=True).strip()
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + f"-{commit}"
    rows = inventory(run_id)
    # Live trial drivers are added only after the static inventory fixes the exact
    # subtype and schema names; this keeps unsupported guesses out of raw data.
    out = RAW / f"{run_id}.jsonl"
    RAW.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8", newline="\n") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
    env = {
        "run_id": run_id,
        "date_utc": now(),
        "commit": commit,
        "cwd": "worktree",
        "python": sys.version.split()[0],
        "versions": {name: version(name) for name in ("claude", "codex")},
        "models": {"claude": "claude-haiku-4-5-20251001", "codex": "gpt-5.6-luna"},
        "model_call_caps": {"claude": 40, "codex": 40},
    }
    (ROOT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
