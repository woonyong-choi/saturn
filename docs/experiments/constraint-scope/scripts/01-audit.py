"""앞 실험의 정답을 다른 모델과 근거 참조로 다시 판정한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
from collections import Counter

from protocol import parse, prompt
from runtime import BASE, MODELS, PRIOR, PRIVATE, PUBLIC, call_cli, read, setup, write


def batches(cases: list[dict]) -> list[list[dict]]:
    groups, group, size = [], [], 0
    for case in cases:
        case_size = len(json.dumps(case, ensure_ascii=False).encode())
        if group and (len(group) >= 4 or size + case_size > 300000):
            groups.append(group)
            group, size = [], 0
        group.append(case)
        size += case_size
    if group:
        groups.append(group)
    return groups


def judge(job: tuple) -> list[dict]:
    cases, phase, model = job
    identity = hashlib.sha256(
        "|".join(c["sample_id"] for c in cases).encode()
    ).hexdigest()[:16]
    trial = f"{phase}-{model}-{identity}"
    record = call_cli("codex", MODELS[model], prompt(cases, phase), trial)
    if BASE.is_rejected(record):
        raise RuntimeError("label model or authentication rejected")
    values = parse(
        record, [c["sample_id"] for c in cases], "query" if phase == "query" else "gold"
    )
    return [
        dict(
            sample_id=c["sample_id"],
            trial_id=trial,
            value=values[i] if values else None,
        )
        for i, c in enumerate(cases)
    ]


def label(cases: list[dict], phase: str, model: str) -> list[dict]:
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        result = [
            row
            for group in pool.map(judge, [(b, phase, model) for b in batches(cases)])
            for row in group
        ]
    write(PRIVATE / f"{phase}-{model}.json", result)
    return result


def main() -> None:
    setup()
    samples, old = read(PRIOR / "samples.json"), read(PRIOR / "gold.json")
    old = {r["sample_id"]: r for r in old if r["label"] != "uncertain"}
    cases = [c for c in samples if c["sample_id"] in old]
    new = label(cases, "audit", "astra")
    counts, differences = Counter(), []
    for row in new:
        previous = old[row["sample_id"]]["label"]
        current = row["value"]["label"] if row["value"] else "invalid"
        counts[previous + "->" + current] += 1
        if previous != current:
            differences.append(
                dict(
                    **row,
                    previous=previous,
                    prior_reasons=[
                        j["value"]["reason"]
                        for j in old[row["sample_id"]]["judgments"]
                        if j["valid"]
                    ],
                )
            )
    write(PRIVATE / "audit-disagreements.json", differences)
    result = dict(
        n=len(cases),
        agreement=sum(v for k, v in counts.items() if len(set(k.split("->"))) == 1),
        transitions=dict(counts),
        disagreement_categories=dict(
            Counter(
                r["value"]["category"] if r["value"] else "invalid" for r in differences
            )
        ),
    )
    write(PUBLIC / "results/audit.json", result)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
