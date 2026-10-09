"""대화 로그만 읽어 기존 세션과 중복을 제외한 표본을 봉인한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import sys
from collections import Counter, deque
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

sys.dont_write_bytecode = True
PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/constraint-registration-generalization"
PRIOR = ROOT / ".local/experiments/constraint-scope/samples.json"
TARGET = 4000
SEED = 38220261005
SPEC = importlib.util.spec_from_file_location(
    "conversation_masking", PUBLIC.parent / "constraint-long-context/scripts/common.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("masking module unavailable")
MASKING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MASKING)


def digest(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def masked(text: str) -> str:
    return MASKING.mask_text(re.sub(r"apikey_[A-Za-z0-9_]+", "[secret]", text))


def save(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def codex_text(payload: dict) -> str:
    return "\n".join(
        item.get("text", "")
        for item in payload.get("content", [])
        if isinstance(item, dict) and item.get("type") in ("input_text", "output_text")
    ).strip()


def injected(text: str) -> bool:
    starts = (
        "# AGENTS.md instructions",
        "<environment_context>",
        "<permissions instructions>",
        "<turn_aborted>",
        "<subagent_notification>",
        "<collaboration_mode>",
        "[Request interrupted",
        "This session is being continued",
    )
    return text.startswith(starts) or MASKING.has_auto_marker(text)


# cost: io 세션 한 번 읽기; basis: estimate
def session_messages(
    path: Path, provider: str, counts: Counter, info: dict
) -> list[dict]:
    messages: list[dict] = []
    seen_ids: set[str] = set()
    with path.open() as stream:
        for line in stream:
            try:
                row = json.loads(line)
            except ValueError:
                counts["invalid_json_lines"] += 1
                continue
            payload = row.get("payload", {})
            if provider == "codex" and row.get("type") == "session_meta":
                if payload.get("source") not in ("vscode", "cli", "app-server"):
                    counts["automatic_codex_sessions"] += 1
                    return []
                info["project"] = payload.get("cwd") or "unknown"
            role, text = None, ""
            if provider == "claude":
                if MASKING.is_human_user_row(row):
                    identity = row.get("uuid")
                    if identity and identity in seen_ids:
                        counts["duplicate_user_ids"] += 1
                        continue
                    if identity:
                        seen_ids.add(identity)
                    role, text = "user", MASKING.get_text(row).strip()
                elif row.get("type") == "assistant" and not row.get("isSidechain"):
                    role, text = "assistant", MASKING.get_text(row).strip()
            elif (
                row.get("type") == "response_item" and payload.get("type") == "message"
            ):
                role = payload.get("role")
                if role in ("user", "assistant"):
                    text = codex_text(payload)
            if role not in ("user", "assistant") or not text:
                continue
            if role == "user" and injected(text):
                counts["injected_user_messages"] += 1
                continue
            messages.append({"role": role, "text": masked(text)})
    return messages


def candidates(
    messages: list[dict], source: str, project: str, provider: str, counts: Counter
) -> list[dict]:
    previous: deque = deque(maxlen=5)
    current: list[dict] = []
    result: list[dict] = []
    for message in messages:
        if message["role"] != "user":
            if current:
                current.append(message)
            continue
        counts["user_inputs"] += 1
        if current:
            previous.append(current)
        state = {
            "previous_context": [item for turn in previous for item in turn],
            "latest_user_input": message["text"],
        }
        current = [message]
        encoded = json.dumps(state, ensure_ascii=False, sort_keys=True)
        if len(encoded.encode()) > 80000:
            counts["oversize_state"] += 1
            continue
        identity = digest(encoded)
        result.append(
            {
                "sample_id": identity,
                "source_id": source,
                "project_id": project,
                "provider": provider,
                "state": state,
                "gold": None,
                "selection_rank": digest(str(SEED) + identity),
            }
        )
    return result


# cost: io 대화 파일 전수 읽기; heap O(n); vars: n = 후보 수; basis: estimate
def main() -> None:
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    if (PRIVATE / "samples.json").exists():
        raise RuntimeError("sample set already exists; refusing overwrite")
    prior = json.loads(PRIOR.read_text())
    excluded_sessions = {r["source_id"] for r in prior}
    excluded_states = {
        digest(json.dumps(r["state"], ensure_ascii=False, sort_keys=True))
        for r in prior
    }
    counts: Counter = Counter()
    unique: dict[str, dict] = {}
    sources: list[dict] = []
    home = Path.home()
    files = [
        ("claude", p) for p in sorted((home / ".claude/projects").glob("*/*.jsonl"))
    ]
    files += [("codex", p) for p in sorted((home / ".codex/sessions").rglob("*.jsonl"))]
    for index, (provider, path) in enumerate(files, 1):
        source = digest(str(path))
        if source[:16] in excluded_sessions:
            counts["prior_sessions"] += 1
            continue
        if "experiment-" in path.parent.name:
            counts["experiment_sessions"] += 1
            continue
        before = path.stat()
        info: dict = {}
        messages = session_messages(path, provider, counts, info)
        after = path.stat()
        if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            counts["changed_during_read"] += 1
            continue
        project = digest(
            path.parent.name if provider == "claude" else info.get("project", "unknown")
        )
        sources.append(
            {
                "source_id": source,
                "provider": provider,
                "source_path": str(path),
                "bytes": before.st_size,
                "mtime_ns": before.st_mtime_ns,
            }
        )
        for case in candidates(messages, source, project, provider, counts):
            sid = case["sample_id"]
            if sid in excluded_states or sid in unique:
                counts["duplicate_or_prior_state"] += 1
                continue
            unique[sid] = case
        if index % 200 == 0:
            print(
                json.dumps({"files_scanned": index, "eligible": len(unique)}),
                flush=True,
            )
    selected = sorted(unique.values(), key=lambda x: x["selection_rank"])[:TARGET]
    for case in selected:
        del case["selection_rank"]
    census = {
        "ts_utc": datetime.now(timezone.utc).isoformat(),
        "seed": SEED,
        "source_files": len(files),
        "eligible": len(unique),
        "selected": len(selected),
        "target": TARGET,
        "shortfall": max(0, TARGET - len(selected)),
        "counts": dict(counts),
        "selected_sessions": len({r["source_id"] for r in selected}),
        "by_provider": dict(Counter(r["provider"] for r in selected)),
        "labels_available": 0,
    }
    save(PRIVATE / "samples.json", selected)
    save(PRIVATE / "sources.json", sources)
    save(PRIVATE / "census.json", census)
    print(json.dumps(census), flush=True)


if __name__ == "__main__":
    main()
