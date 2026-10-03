"""Claude Code 기록을 읽기 전용으로 추출하고 가린 private 표본을 만든다."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (  # noqa: E402
    PRIVATE_ROOT,
    get_text,
    is_human_user_row,
    mask_text,
    sha256_file,
    write_json,
    write_jsonl,
    ensure_private_root,
    extract_paths,
)

SOURCE_ROOT = Path.home() / ".claude" / "projects"
SEED = 316
BANDS = ((30, 119, "30-119"), (120, 399, "120-399"), (400, None, "400+"))


def parse_file(path: Path) -> list[dict]:
    turns = []
    with path.open(encoding="utf-8") as file:
        for line_number, line in enumerate(file, start=1):
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not is_human_user_row(row):
                continue
            text = get_text(row).strip()
            turns.append({
                "turn_id": f"u-{len(turns) + 1:04d}",
                "turn_index": len(turns),
                "timestamp": row.get("timestamp") or row.get("message", {}).get("timestamp"),
                "text": mask_text(text),
                "source_line": line_number,
                "files": extract_paths(text),
            })
    return turns


def band_for(count: int) -> str | None:
    for low, high, name in BANDS:
        if count >= low and (high is None or count <= high):
            return name
    return None


def sample_indexes(count: int, limit: int = 20) -> set[int]:
    if count <= limit:
        return set(range(count))
    indexes = set(range(5)) | set(range(count - 5, count))
    for step in range(10):
        indexes.add(round(5 + step * (count - 11) / 9))
    return set(sorted(indexes))


def select_files(files: list[Path]) -> list[tuple[Path, list[dict], str]]:
    candidates = []
    for path in files:
        turns = parse_file(path)
        band = band_for(len(turns))
        if band:
            candidates.append((path, turns, band))
    chosen = []
    for _, _, band in BANDS:
        members = [row for row in candidates if row[2] == band]
        members.sort(key=lambda row: (
            0 if "workspace/oss/saturn" in str(row[0]) else 1,
            hashlib.sha256(str(row[0]).encode("utf-8")).hexdigest(),
        ))
        chosen.extend(members[:2])
    return chosen


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.dry_run:
        print("추출 드라이런 통과: source를 읽지 않고 표본·가림 상수만 검사")
        return 0
    ensure_private_root()
    files = sorted(SOURCE_ROOT.glob("*/*.jsonl"))
    selected = select_files(files)
    if not selected:
        raise RuntimeError("30개 이상 사용자 턴을 가진 Claude Code 대화를 찾지 못했다")
    now = dt.datetime.now(dt.timezone.utc)
    run_id = f"{now.strftime('%Y%m%dT%H%M%SZ')}-{sha256_file(Path(__file__))[:7]}"
    rows = []
    metadata = []
    for path, turns, band in selected:
        conversation_id = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
        sampled = sample_indexes(len(turns))
        metadata.append({
            "conversation_id": conversation_id,
            "length_band": band,
            "user_turn_count": len(turns),
            "sampled_turn_count": len(sampled),
            "source_sha256": sha256_file(path),
            "is_saturn": "workspace/oss/saturn" in str(path),
        })
        for index, turn in enumerate(turns):
            rows.append({
                "run_id": run_id,
                "conversation_id": conversation_id,
                "length_band": band,
                "turn_id": turn["turn_id"],
                "turn_index": index,
                "ts_utc": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
                "timestamp": turn["timestamp"],
                "text": turn["text"],
                "files": turn["files"],
                "sampled": index in sampled,
                "source_path": str(path),
            })
    write_jsonl(PRIVATE_ROOT / "selected.jsonl", rows)
    write_json(PRIVATE_ROOT / "selection.json", {
        "run_id": run_id,
        "seed": SEED,
        "source_root": str(SOURCE_ROOT),
        "conversations": metadata,
    })
    print(f"추출 끝: 대화 {len(metadata)}개, 전체 사용자 턴 {len(rows)}개, Jev 표본 {sum(row['sampled'] for row in rows)}개")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
