"""수집 당시 파일 해시와 접두 바이트로 중복 제외 수를 복원한다."""

from __future__ import annotations

import hashlib
import json
import re
from collections import Counter
from pathlib import Path

from support import BASE, PRIVATE, load, read, write


def main() -> None:
    extractor = load("audit_extractor", "01-collect.py")
    original = read(BASE / "sample.json")
    ids = {c["id"] for c in original}
    seen = {(c["project"], c["uuid"]) for c in original}
    counts = Counter()
    for meta in read(PRIVATE / "extension-census.json")["files"]:
        path = Path(meta["source_path"])
        data = path.read_bytes()[: meta["bytes"]]
        if hashlib.sha256(data).hexdigest() != meta["sha256"]:
            counts["unreconstructable_files"] += 1
            continue
        counts["verified_files"] += 1
        prior = False
        project = extractor.digest(extractor.project_key(path.parent.name))
        for line_no, line in enumerate(data.splitlines(), 1):
            try:
                row = json.loads(line)
            except (ValueError, UnicodeDecodeError):
                continue
            if not extractor.previous.is_human_user_row(row):
                continue
            text = extractor.mask(extractor.previous.get_text(row).strip())
            if not prior:
                prior = True
                continue
            if not re.search("[가-힣]", text):
                continue
            counts["eligible_before_dedup"] += 1
            case_id = extractor.digest(str(path) + ":" + str(line_no))
            key = (project, row.get("uuid"))
            if case_id in ids:
                counts["existing_id_excluded"] += 1
            elif key[1] and key in seen:
                counts["uuid_duplicate_excluded"] += 1
            else:
                seen.add(key)
                counts["extra_eligible"] += 1
    output = {
        key: counts[key]
        for key in (
            "verified_files",
            "unreconstructable_files",
            "eligible_before_dedup",
            "existing_id_excluded",
            "uuid_duplicate_excluded",
            "extra_eligible",
        )
    }
    write(PRIVATE / "extraction-audit.json", output)
    print(json.dumps(output))


if __name__ == "__main__":
    main()
