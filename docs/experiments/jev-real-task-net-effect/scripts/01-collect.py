"""실제 과제 source 가용성·봉인 사전 검사. provider 호출은 아직 없다."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[2]
PRIVATE = ROOT / ".runtime/jev-real-task-effect"
MANIFEST = PRIVATE / "manifest.json"
SEAL = PRIVATE / "seal.json"
RAW = PRIVATE / "raw"
DESIGN_FILES = (
    "docs/experiments/jev-real-task-net-effect/design.md",
    "docs/experiments/jev-real-task-net-effect/run.sh",
    "docs/experiments/jev-real-task-net-effect/scripts/01-collect.py",
)
CASE_FIELDS = {
    "task_id", "repository_url", "issue_url", "issue_at", "base_commit",
    "source_home", "source_work", "source_chat", "source_sha256",
    "goal_text", "goal_sha256", "candidate_sha256", "policy_sha256",
    "provider_direction", "target_model", "safety_percent", "repair_prompt",
    "check_command", "check_cwd",
}


def encoded(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def tree_digest(root: Path) -> str:
    if not root.is_dir():
        raise ValueError("source snapshot directory missing")
    rows = []
    for item in sorted(root.rglob("*")):
        rel = item.relative_to(root).as_posix()
        if item.name == "engine.sock" or item.name.endswith(".lock") or "packet-capture" in item.parts:
            continue
        if item.is_symlink():
            rows.append([rel, "link", os.readlink(item)])
        elif item.is_file():
            rows.append([rel, "file", digest(item.read_bytes())])
    return digest(encoded(rows))


def source_digest(case: dict) -> str:
    return digest(encoded({"home": tree_digest(Path(case["source_home"])), "work": tree_digest(Path(case["source_work"]))}))


def committed_design() -> str:
    for path in DESIGN_FILES:
        subprocess.check_output(["git", "ls-files", "--error-unmatch", path], cwd=ROOT, stderr=subprocess.DEVNULL)
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--", *DESIGN_FILES], cwd=ROOT)
    if dirty:
        raise ValueError("design and preflight script must be committed")
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()


def load_manifest() -> dict:
    value = json.loads(MANIFEST.read_text())
    cases, phase = value.get("cases"), value.get("phase")
    if value.get("schema") != 1 or value.get("seed") != 7007:
        raise ValueError("manifest schema or seed mismatch")
    if not isinstance(cases, list) or not ((phase == "diagnostic" and 1 <= len(cases) <= 8) or (phase == "confirmatory" and len(cases) == 80)):
        raise ValueError("diagnostic requires 1-8 cases; confirmatory requires 80")
    if not value.get("eligibility_audit_sha256") or not value.get("pricing") or not value.get("environment"):
        raise ValueError("eligibility audit, pricing, and environment required")
    ids = set()
    directions = {"claude-codex": 0, "codex-claude": 0}
    repositories = {}
    for case in cases:
        if CASE_FIELDS - case.keys():
            raise ValueError("case fields missing")
        task_id = case["task_id"]
        if not isinstance(task_id, str) or not task_id.isascii() or not task_id.replace("-", "").isalnum() or task_id in ids:
            raise ValueError("invalid or duplicate task id")
        ids.add(task_id)
        direction = case["provider_direction"]
        if direction not in directions:
            raise ValueError("invalid direction")
        directions[direction] += 1
        expected = "gpt-5.6-sol" if direction == "claude-codex" else "sonnet"
        if case["target_model"] != expected or not 1 <= int(case["safety_percent"]) <= 100:
            raise ValueError("model or safety percent differs from design")
        if digest(case["goal_text"].encode()) != case["goal_sha256"] or source_digest(case) != case["source_sha256"]:
            raise ValueError("goal or source snapshot hash mismatch")
        repo = case["repository_url"]
        repositories[repo] = repositories.get(repo, 0) + 1
        if repositories[repo] > 20 or not case["check_command"] or not case["repair_prompt"]:
            raise ValueError("repository quota or task instructions invalid")
    if phase == "confirmatory" and (directions != {"claude-codex": 40, "codex-claude": 40} or len(repositories) < 4):
        raise ValueError("confirmatory quota mismatch")
    if [case["task_id"] for case in cases] != sorted(ids, key=lambda item: digest(item.encode())):
        raise ValueError("cases must be sorted by task-id SHA-256")
    return value


def inventory() -> dict:
    if not MANIFEST.is_file():
        result = {"manifest_present": False, "real_sources": 0, "runnable_pairs": 0, "provider_calls": 0}
    else:
        try:
            value = load_manifest()
            result = {"manifest_present": True, "real_sources": len(value["cases"]), "runnable_pairs": 0, "provider_calls": 0}
        except (ValueError, KeyError, OSError, json.JSONDecodeError) as error:
            result = {"manifest_present": True, "real_sources": 0, "runnable_pairs": 0, "provider_calls": 0, "preflight_error": type(error).__name__}
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return result


def seal() -> None:
    head = committed_design()
    if SEAL.exists() or RAW.exists():
        raise ValueError("existing seal or raw data; overwrite prohibited")
    value = load_manifest()
    binary = Path(value["environment"]["engine_binary"])
    if not binary.is_file() or digest(binary.read_bytes()) != value["environment"]["engine_sha256"]:
        raise ValueError("engine binary missing or hash mismatch")
    SEAL.write_bytes(encoded({"created_at": datetime.now(timezone.utc).isoformat(), "experiment_commit": head, "manifest_sha256": digest(MANIFEST.read_bytes()), "engine_sha256": value["environment"]["engine_sha256"], "phase": value["phase"], "case_count": len(value["cases"])}))


def verify() -> None:
    head = committed_design()
    if not SEAL.is_file():
        raise ValueError("seal missing")
    value = json.loads(SEAL.read_text())
    if value["experiment_commit"] != head or digest(MANIFEST.read_bytes()) != value["manifest_sha256"]:
        raise ValueError("commit or manifest changed after seal")
    load_manifest()


def collect() -> None:
    if inventory()["real_sources"] == 0:
        raise ValueError("no sealed real-task source snapshots; provider calls: 0")
    raise ValueError("raw collector is not implemented; provider calls: 0")


def not_collected() -> None:
    raise ValueError("raw collection has not run")


def main() -> None:
    commands = {"inventory": inventory, "seal": seal, "collect": collect, "process": not_collected, "analyze": not_collected, "verify": verify}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        raise SystemExit("usage: 01-collect.py inventory|seal|collect|process|analyze|verify")
    try:
        commands[sys.argv[1]]()
    except (ValueError, KeyError, OSError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"preflight failed: {error}") from None


if __name__ == "__main__":
    main()
