"""기존 추출·가림 규칙으로 같은 세션의 앞뒤 대화가 있는 표본을 고정한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import random
import re
from collections import Counter, defaultdict
from pathlib import Path

from runtime import PRIVATE, PUBLIC, SEED, read, redact, setup, write

SPEC = importlib.util.spec_from_file_location(
    "masking", PUBLIC.parent / "constraint-long-context/scripts/common.py"
)
MASKING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MASKING)
BANDS = ("10-29", "30-119", "120-399", "400+")


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()[:16]


def project_key(name: str) -> str:
    normalized = name.lower()
    prefix = "-users-woonyong-workspace-oss-saturn"
    if normalized.startswith(prefix) and not normalized.startswith(prefix + "-cli"):
        return "saturn"
    if normalized.startswith("-users-woonyong-orca-workspaces-saturn-docs-sync"):
        return "saturn"
    return normalized


def band_for(count: int) -> str | None:
    if count < 10:
        return None
    return next(
        name
        for upper, name in zip((29, 119, 399, float("inf")), BANDS)
        if count <= upper
    )


def input_kind(text: str) -> str:
    headings = len(re.findall(r"(?m)^#{1,6}\s", text))
    directives = len(re.findall(r"(?m)^\s*(?:[-*]|\d+[.)])\s", text))
    handoff = bool(
        re.search(
            r"다음 (?:세션|대화|에이전트)|이어서 작업|작업 지시|완료 조건|인수인계|handoff",
            text,
            re.I,
        )
    )
    structured = len(text) >= 500 and headings >= 2 and directives >= 3
    return (
        "ai_instruction_likely"
        if structured or (len(text) >= 300 and handoff and directives >= 3)
        else "human_likely"
    )


# cost: time O(b), heap O(t), io 1 source read; vars: b = bytes, t = text; basis: estimate
def extract(path: Path) -> tuple[list[dict], dict]:
    data = path.read_bytes()
    turns, invalid, seen = [], 0, set()
    for line_number, line in enumerate(data.splitlines(), 1):
        try:
            row = json.loads(line)
        except ValueError:
            invalid += 1
            continue
        if MASKING.is_human_user_row(row):
            uuid = row.get("uuid") or str(line_number)
            if uuid in seen:
                continue
            seen.add(uuid)
            text = MASKING.mask_text(redact(MASKING.get_text(row).strip()))
            turns.append(
                dict(
                    uuid=uuid,
                    source_line=line_number,
                    user=text,
                    assistant=[],
                    kind=input_kind(text),
                )
            )
        elif turns and row.get("type") == "assistant" and not row.get("isSidechain"):
            text = MASKING.get_text(row).strip()
            if text:
                turns[-1]["assistant"].append(MASKING.mask_text(redact(text)))
    meta = dict(
        source_path=str(path),
        source_id=digest(str(path)),
        sha256=hashlib.sha256(data).hexdigest(),
        project=project_key(path.parent.name),
        turns=len(turns),
        invalid_lines=invalid,
    )
    return turns, meta


def messages(turns: list[dict]) -> list[dict]:
    result = []
    for turn in turns:
        result.append(dict(role="user", text=turn["user"]))
        result.extend(dict(role="assistant", text=text) for text in turn["assistant"])
    return result


def make_cases(
    turns: list[dict], meta: dict, count: int, excluded: Counter
) -> list[dict]:
    band = band_for(count)
    if band is None:
        excluded["project_under_10"] += len(turns)
        return []
    cases = []
    for index, turn in enumerate(turns):
        if len(turns) - index - 1 < 3:
            excluded["fewer_than_3_future_users"] += 1
            continue
        future_turns = turns[index + 1 : index + 6]
        future = [dict(role="assistant", text=t) for t in turn["assistant"]] + messages(
            future_turns
        )
        if not any(item["role"] == "assistant" for item in future):
            excluded["no_future_assistant"] += 1
            continue
        state = dict(
            previous_context=messages(turns[max(0, index - 5) : index]),
            latest_user_input=turn["user"],
        )
        if (
            len(json.dumps(state, ensure_ascii=False).encode()) > 80000
            or len(json.dumps(future, ensure_ascii=False).encode()) > 180000
        ):
            excluded["context_size"] += 1
            continue
        cases.append(
            dict(
                sample_id=digest(meta["source_id"] + turn["uuid"]),
                source_id=meta["source_id"],
                source_line=turn["source_line"],
                uuid=turn["uuid"],
                project_id=digest(meta["project"]),
                length_band=band,
                input_kind=turn["kind"],
                state=state,
                future=future,
                future_user_count=len(future_turns),
            )
        )
    return cases


def select(cases: list[dict]) -> list[dict]:
    rng = random.Random(SEED)
    strata = defaultdict(list)
    for case in cases:
        strata[(case["length_band"], case["project_id"], case["input_kind"])].append(
            case
        )
    for group in strata.values():
        rng.shuffle(group)
    keys = sorted(strata)
    rng.shuffle(keys)
    selected = []
    while len(selected) < 100 and any(strata.values()):
        for key in keys:
            if strata[key] and len(selected) < 100:
                selected.append(strata[key].pop())
    if len(selected) != 100 or len({c["input_kind"] for c in selected}) != 2:
        raise RuntimeError("insufficient eligible sample or input kinds")
    rng.shuffle(selected)
    return selected


# cost: io all session files once; memory proportional to extracted text; basis: estimate
def main() -> None:
    setup()
    if (PRIVATE / "samples.json").exists():
        print(json.dumps(read(PRIVATE / "sampling.json")))
        return
    extracted, census = [], []
    project_counts, excluded = Counter(), Counter()
    for path in sorted((Path.home() / ".claude/projects").glob("*/*.jsonl")):
        if "experiment-382-model-compare" in path.parent.name:
            continue
        turns, meta = extract(path)
        extracted.append((turns, meta))
        census.append(meta)
        project_counts[meta["project"]] += len(turns)
    cases, seen = [], set()
    for turns, meta in extracted:
        for case in make_cases(turns, meta, project_counts[meta["project"]], excluded):
            key = (case["project_id"], case["uuid"])
            if key in seen:
                excluded["duplicate_uuid"] += 1
                continue
            seen.add(key)
            cases.append(case)
    selected = select(cases)
    stats = dict(
        seed=SEED,
        files=len(census),
        extracted_users=sum(m["turns"] for m in census),
        eligible=len(cases),
        excluded=dict(excluded),
        selected=len(selected),
        selected_projects=len({c["project_id"] for c in selected}),
        selected_sessions=len({c["source_id"] for c in selected}),
        by_kind=dict(Counter(c["input_kind"] for c in selected)),
        by_band=dict(Counter(c["length_band"] for c in selected)),
        eligible_by_kind=dict(Counter(c["input_kind"] for c in cases)),
        min_future_users=min(c["future_user_count"] for c in selected),
    )
    write(PRIVATE / "census.json", census)
    write(PRIVATE / "samples.json", selected)
    write(PRIVATE / "sampling.json", stats)
    write(
        PRIVATE / "sample-seal.json",
        {
            name: hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest()
            for name in ("census.json", "samples.json", "sampling.json")
        },
    )
    print(json.dumps(stats))


if __name__ == "__main__":
    main()
