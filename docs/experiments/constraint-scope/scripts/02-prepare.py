"""기존 표본과 추가 대화를 프로젝트 및 입력 형태로 층화한다."""

from __future__ import annotations

import hashlib
import json
import random
from collections import Counter, defaultdict
from pathlib import Path

from runtime import PRIOR, PRIVATE, PUBLIC, SEED, load_module, read, setup, write

EXTRACT = load_module(
    "previous_prepare", PUBLIC.parent / "constraint-model-compare/scripts/01-prepare.py"
)
TARGET = 400


def main() -> None:
    setup()
    if (PRIVATE / "samples.json").exists():
        print(json.dumps(read(PRIVATE / "sampling.json")))
        return
    if not (PRIVATE / "design-seal.json").exists():
        raise RuntimeError("design seal required")
    old = read(PRIOR / "samples.json")
    selected = [dict(c, previously_seen=True) for c in old]
    seen = {(c["project_id"], c["uuid"]) for c in old}
    extracted, census, project_uuids = [], [], defaultdict(set)
    for path in sorted((Path.home() / ".claude/projects").glob("*/*.jsonl")):
        if "experiment-382-" in path.parent.name:
            continue
        turns, meta = EXTRACT.extract(path)
        census.append(meta)
        extracted.append((turns, meta))
        project_uuids[meta["project"]].update(t["uuid"] for t in turns)
    excluded, strata = Counter(), defaultdict(list)
    for turns, meta in extracted:
        for case in EXTRACT.make_cases(
            turns, meta, len(project_uuids[meta["project"]]), excluded
        ):
            key = (case["project_id"], case["uuid"])
            if key in seen:
                excluded["previous_or_duplicate"] += 1
                continue
            seen.add(key)
            strata[
                (case["length_band"], case["project_id"], case["input_kind"])
            ].append(dict(case, previously_seen=False))
    rng = random.Random(SEED)
    for group in strata.values():
        rng.shuffle(group)
    keys = sorted(strata)
    rng.shuffle(keys)
    eligible = len(selected) + sum(map(len, strata.values()))
    while len(selected) < TARGET and any(strata.values()):
        for key in keys:
            if strata[key] and len(selected) < TARGET:
                selected.append(strata[key].pop())
    if len(selected) != TARGET:
        raise RuntimeError("insufficient eligible samples")
    rng.shuffle(selected)
    stats = dict(
        seed=SEED,
        files=len(census),
        eligible=eligible,
        selected=len(selected),
        prior=100,
        new=len(selected) - 100,
        excluded=dict(excluded),
        projects=len({c["project_id"] for c in selected}),
        sessions=len({c["source_id"] for c in selected}),
        by_kind=dict(Counter(c["input_kind"] for c in selected)),
        by_project_kind=dict(
            Counter(c["project_id"] + ":" + c["input_kind"] for c in selected)
        ),
        by_band=dict(Counter(c["length_band"] for c in selected)),
    )
    write(PRIVATE / "samples.json", selected)
    write(PRIVATE / "census.json", census)
    write(PRIVATE / "sampling.json", stats)
    write(
        PRIVATE / "sample-seal.json",
        {
            n: hashlib.sha256((PRIVATE / n).read_bytes()).hexdigest()
            for n in ("samples.json", "census.json", "sampling.json")
        },
    )
    print(json.dumps(stats))


if __name__ == "__main__":
    main()
