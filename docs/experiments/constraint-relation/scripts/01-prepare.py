"""읽기 전용 원자료 목록과 예산 안의 무작위 표본을 봉인한다."""

from __future__ import annotations

import random

from support import (
    PRIVATE,
    SEED,
    SOURCE,
    initialize,
    load_original,
    read_json,
    source_manifest,
    write_json,
)


def main() -> None:
    initialize()
    if (PRIVATE / "plan.json").exists():
        return
    conversations, _, _ = load_original()
    population = [
        (c["conversation_id"], t["turn_id"])
        for c in sorted(conversations, key=lambda c: c["conversation_id"])
        for t in c["turns"]
    ]
    chosen = random.Random(SEED).sample(population, 400)
    projects = read_json(SOURCE / "selection.json")["projects"]
    short_ids = sorted(
        m["conversation_id"] for m in projects if 0 < m["user_turns"] < 10
    )
    groups, batch, count = [], [], 0
    for cid in short_ids:
        c = read_json(SOURCE / "conversations" / (cid + ".json"))
        if count + len(c["turns"]) > 40:
            groups.append(batch)
            batch, count = [], 0
        batch.append(cid)
        count += len(c["turns"])
    if batch:
        groups.append(batch)
    write_json(
        PRIVATE / "plan.json",
        {
            "seed": SEED,
            "sample": chosen,
            "label_batches": groups,
            "source_manifest": source_manifest(),
        },
    )
    print(
        {
            "sample_turns": len(chosen),
            "label_batches": len(groups),
            "short_projects": len(short_ids),
        }
    )


if __name__ == "__main__":
    main()
