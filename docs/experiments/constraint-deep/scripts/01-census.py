"""원본을 바꾸지 않고 파일별 사용자 턴 수와 기간을 전수 조사한다."""

from __future__ import annotations

import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path

from storage import (
    PRIVATE,
    band_for,
    digest,
    ensure_private,
    is_human_user_row,
    now,
    write_json,
)


# cost: time O(b), heap O(l), io f files; vars: b = bytes, l = longest line, f = files; basis: estimate
def inspect_file(path: Path) -> dict:
    count, invalid = 0, 0
    timestamps = []
    checksum = hashlib.sha256()
    for line in path.open("rb"):
        checksum.update(line)
        try:
            row = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            invalid += 1
            continue
        if not is_human_user_row(row):
            continue
        count += 1
        timestamp = row.get("timestamp") or row.get("message", {}).get("timestamp")
        if timestamp:
            timestamps.append(str(timestamp))
    return {
        "file_id": digest(str(path)),
        "source_path": str(path),
        "project_id": digest(path.parent.name),
        "project": path.parent.name,
        "is_saturn": "workspace-oss-saturn" in path.parent.name,
        "user_turns": count,
        "length_band": band_for(count),
        "start": min(timestamps, default=None),
        "end": max(timestamps, default=None),
        "missing_timestamps": count - len(timestamps),
        "invalid_lines": invalid,
        "source_sha256": checksum.hexdigest(),
        "bytes": path.stat().st_size,
    }


def main() -> None:
    ensure_private()
    target = PRIVATE / "census.json"
    if target.exists():
        raise RuntimeError("census already frozen")
    paths = sorted((Path.home() / ".claude/projects").glob("*/*.jsonl"))
    files = [inspect_file(path) for path in paths]
    projects = defaultdict(int)
    for row in files:
        projects[row["project_id"]] += row["user_turns"]
    summary = {
        "files": len(files),
        "bytes": sum(row["bytes"] for row in files),
        "user_turns": sum(row["user_turns"] for row in files),
        "projects": len(projects),
        "session_bands": dict(Counter(row["length_band"] or "<10" for row in files)),
        "project_bands": dict(
            Counter(band_for(count) or "<10" for count in projects.values())
        ),
        "invalid_lines": sum(row["invalid_lines"] for row in files),
        "missing_timestamps": sum(row["missing_timestamps"] for row in files),
        "saturn_turns": sum(row["user_turns"] for row in files if row["is_saturn"]),
    }
    write_json(target, {"ts_utc": now(), "files": files, "summary": summary})
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
