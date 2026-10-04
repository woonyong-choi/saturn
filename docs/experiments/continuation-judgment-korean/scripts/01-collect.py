"""원본을 읽고 같은 세션의 직전 작업과 한국어 입력을 층화 추출한다."""

from __future__ import annotations

import hashlib
import json
import random
import re
from collections import Counter, defaultdict
from pathlib import Path

from storage import PRIVATE, SEED, digest, mask, now, previous, setup, write


def project_key(name: str) -> str:
    name = name.lower()
    root = "-users-woonyong-workspace-oss-saturn"
    if name.startswith(root) and not name.startswith(root + "-cli"):
        return "saturn"
    if name.startswith("-users-woonyong-orca-workspaces-saturn-docs-sync"):
        return "saturn"
    return name


def extract(path: Path, counts: Counter) -> tuple[list[dict], dict]:
    data = path.read_bytes()
    before = hashlib.sha256(data).hexdigest()
    project = project_key(path.parent.name)
    cases, prior, texts, stop = [], None, [], None
    assistant_count = 0
    for line_no, line in enumerate(data.splitlines(), 1):
        try:
            row = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            counts["invalid_json"] += 1
            continue
        if row.get("type") == "assistant" and not row.get("isSidechain"):
            text = previous.get_text(row).strip()
            if text:
                texts.append(mask(text))
            stop = row.get("message", {}).get("stop_reason")
            assistant_count += 1
        if not previous.is_human_user_row(row):
            if row.get("type") == "user":
                counts["non_human_user_rows"] += 1
            continue
        counts["human_turns"] += 1
        text = mask(previous.get_text(row).strip())
        if prior is None:
            counts["first_turn"] += 1
        elif not re.search("[가-힣]", text):
            counts["non_korean"] += 1
        else:
            progress = "\n".join(texts[-3:])
            cases.append(
                {
                    "id": digest(str(path) + ":" + str(line_no)),
                    "project": digest(project),
                    "is_saturn": project == "saturn",
                    "session": digest(str(path)),
                    "source_line": line_no,
                    "uuid": row.get("uuid"),
                    "timestamp": row.get("timestamp"),
                    "input": text,
                    "previous_input": prior,
                    "goal_excerpt": prior[:2000],
                    "progress_excerpt": progress[-6000:],
                    "goal_omitted_chars": max(0, len(prior) - 2000),
                    "progress_omitted_chars": max(
                        0, len("\n".join(texts)) - len(progress[-6000:])
                    ),
                    "running": stop != "end_turn",
                    "stop_reason": stop,
                    "assistant_count": assistant_count,
                    "previous_disposition": "queue",
                }
            )
        prior, texts, stop, assistant_count = text, [], None, 0
    if hashlib.sha256(path.read_bytes()).hexdigest() != before:
        counts["changed_files"] += 1
        cases = []
    return cases, {
        "source_path": str(path),
        "sha256": before,
        "bytes": len(data),
        "session": digest(str(path)),
    }


def main() -> None:
    setup()
    if (PRIVATE / "sample.json").exists():
        print("using frozen sample")
        return
    counts, groups, files, seen = Counter(), defaultdict(list), [], set()
    for path in sorted((Path.home() / ".claude/projects").glob("*/*.jsonl")):
        cases, meta = extract(path, counts)
        files.append(meta)
        for case in cases:
            key = (case["project"], case["uuid"])
            if case["uuid"] and key in seen:
                counts["duplicate_uuid"] += 1
                continue
            seen.add(key)
            groups[case["project"]].append(case)
    total = sum(map(len, groups.values()))
    target = min(660, total)
    if not any(c["is_saturn"] for group in groups.values() for c in group):
        raise RuntimeError("no eligible saturn cases")
    if len(groups) > target:
        raise RuntimeError("more strata than sample capacity")
    quotas = {p: max(1, int(target * len(g) / total)) for p, g in groups.items()}
    while sum(quotas.values()) != target:
        if sum(quotas.values()) < target:
            p = max(
                (p for p in groups if quotas[p] < len(groups[p])),
                key=lambda p: (target * len(groups[p]) / total - quotas[p], p),
            )
            quotas[p] += 1
        else:
            p = max(
                (p for p in groups if quotas[p] > 1),
                key=lambda p: (quotas[p] - target * len(groups[p]) / total, p),
            )
            quotas[p] -= 1
    rng, selected = random.Random(SEED), []
    for p in sorted(groups):
        rng.shuffle(groups[p])
        selected.extend(groups[p][: quotas[p]])
    rng.shuffle(selected)
    for case in selected:
        case["condition_order"] = rng.sample(["A", "B"], 2)
    write(
        PRIVATE / "census.json",
        {"ts_utc": now(), "files": files, "counts": dict(counts)},
    )
    write(PRIVATE / "population.json", [c for p in sorted(groups) for c in groups[p]])
    write(PRIVATE / "sample.json", selected)
    selection = {
        "counts": dict(counts),
        "files": len(files),
        "eligible": total,
        "selected": len(selected),
        "strata": {
            p: {"population": len(groups[p]), "sample": quotas[p]}
            for p in sorted(groups)
        },
    }
    write(PRIVATE / "selection.json", selection)
    print(json.dumps(selection))


if __name__ == "__main__":
    main()
