"""고정 목록에서 가린 프로젝트 대화와 전수 표본을 만든다."""

from __future__ import annotations

import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path

from storage import (
    BANDS,
    PRIVATE,
    band_for,
    digest,
    ensure_private,
    get_text,
    is_human_user_row,
    mask_text,
    previous,
    read_json,
    write_json,
)


def project_key(name: str) -> str:
    normalized = name.lower()
    saturn_root = "-users-woonyong-workspace-oss-saturn"
    if normalized.startswith(saturn_root) and not normalized.startswith(
        saturn_root + "-cli"
    ):
        return "saturn"
    if normalized.startswith("-users-woonyong-orca-workspaces-saturn-docs-sync"):
        return "saturn"
    return normalized


def extract_turns(meta: dict) -> list[dict]:
    turns = []
    for line_number, line in enumerate(Path(meta["source_path"]).open(), 1):
        try:
            row = json.loads(line)
        except ValueError:
            continue  # 전수 목록에 이미 잘못된 줄 수를 기록한다.
        if not is_human_user_row(row):
            continue
        text = get_text(row).strip()
        turns.append(
            {
                "source_id": meta["file_id"],
                "source_line": line_number,
                "uuid": row.get("uuid"),
                "text": mask_text(text),
                "files": previous.extract_paths(text),
                "timestamp": row.get("timestamp")
                or row.get("message", {}).get("timestamp"),
            }
        )
    return turns


def build_conversation(files: list[dict], key: str) -> dict:
    turns, seen, duplicates = [], set(), 0
    for meta in sorted(files, key=lambda row: (row["start"] or "", row["file_id"])):
        for row in extract_turns(meta):
            if row["uuid"] and row["uuid"] in seen:
                duplicates += 1
                continue
            if row["uuid"]:
                seen.add(row["uuid"])
            row["turn_id"] = f"u-{len(turns) + 1:04d}"
            row["turn_index"] = len(turns)
            turns.append(row)
    return {
        "conversation_id": digest(key),
        "is_saturn": key == "saturn",
        "length_band": band_for(len(turns)),
        "user_turns": len(turns),
        "session_count": len(files),
        "duplicate_turns": duplicates,
        "turns": turns,
    }


def main() -> None:
    ensure_private()
    if (PRIVATE / "selection.json").exists():
        raise RuntimeError("selection already frozen")
    census = read_json(PRIVATE / "census.json")
    grouped = defaultdict(list)
    changed = []
    for meta in census["files"]:
        actual = hashlib.sha256(Path(meta["source_path"]).read_bytes()).hexdigest()
        if actual != meta["source_sha256"]:
            changed.append(meta["file_id"])
            continue
        grouped[project_key(meta["project"])].append(meta)
    summaries = []
    for key, files in sorted(grouped.items()):
        conversation = build_conversation(files, key)
        write_json(
            PRIVATE / "conversations" / (conversation["conversation_id"] + ".json"),
            conversation,
        )
        summaries.append({k: v for k, v in conversation.items() if k != "turns"})
    selected = [row for row in summaries if row["length_band"] in BANDS]
    selection = {
        "census": census["summary"],
        "changed_files": changed,
        "projects": summaries,
        "selected": selected,
        "conversations_by_band": dict(Counter(row["length_band"] for row in selected)),
        "turns_by_band": {
            band: sum(
                row["user_turns"] for row in selected if row["length_band"] == band
            )
            for band in BANDS
        },
    }
    write_json(PRIVATE / "selection.json", selection)
    print(
        json.dumps(
            {
                k: selection[k]
                for k in ("conversations_by_band", "turns_by_band", "changed_files")
            }
        )
    )


if __name__ == "__main__":
    main()
