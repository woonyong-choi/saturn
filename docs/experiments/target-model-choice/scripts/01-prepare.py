"""사용자 입력만 추출해 세션 첫 작업을 종류와 프로젝트로 층화한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import random
import re
from collections import Counter, defaultdict
from pathlib import Path

from runtime import PRIVATE, PUBLIC, SEED, digest, read, redact, setup, write

SPEC = importlib.util.spec_from_file_location(
    "masking", PUBLIC.parent / "constraint-long-context/scripts/common.py"
)
MASK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MASK)


def hash_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def category(text: str) -> str:
    groups = [
        ("debugging", r"버그|오류|에러|실패|debug|error|broken|작동.*않|안\s*돼"),
        ("research", r"조사|비교|찾아|검색|research|찾아봐|알아봐"),
        ("design_docs", r"설계|문서|정리|작성|글|아키텍처|계획|design|README"),
        ("code_change", r"구현|수정|코드|리팩터|개발|만들|추가|기능|implement|fix"),
        ("simple_question", r"뭐|무엇|어떻게|왜|설명|\?|알려|what|how|why"),
    ]
    return next(
        (name for name, pattern in groups if re.search(pattern, text, re.I)), "other"
    )


def project_id(name: str) -> str:
    name = name.lower().replace("/", "-").replace(".", "-")
    if "workspace-oss-saturn" in name and "saturn-cli" not in name:
        name = "saturn"
    return hash_text(name)


def codex_user(row: dict) -> str | None:
    if row.get("type") != "response_item":
        return None
    item = row.get("payload", {})
    if item.get("type") != "message" or item.get("role") != "user":
        return None
    content = item.get("content", [])
    return "\n".join(
        b.get("text", "") for b in content if b.get("type") in ("input_text", "text")
    )


def is_content(text: str) -> bool:
    prefixes = (
        "# AGENTS.md instructions",
        "<environment_context>",
        "<permissions instructions>",
        "<turn_aborted>",
        "<subagent_notification>",
        "<user_instructions>",
    )
    return (
        bool(text) and not text.startswith(prefixes) and not MASK.has_auto_marker(text)
    )


# cost: time O(b), heap O(u), io 1 session read; vars: b = bytes, u = user text; basis: estimate
def extract(path: Path, source: str) -> tuple[list[dict], dict]:
    turns, seen = [], set()
    checksum = hashlib.sha256()
    project = path.parent.name if source == "claude" else "unknown"
    invalid = 0
    for number, line in enumerate(path.open("rb"), 1):
        checksum.update(line)
        try:
            row = json.loads(line)
        except ValueError:
            invalid += 1
            continue
        if source == "codex" and row.get("type") == "session_meta":
            project = row.get("payload", {}).get("cwd", "unknown")
        if source == "claude":
            text = MASK.get_text(row).strip() if MASK.is_human_user_row(row) else None
        else:
            text = codex_user(row)
            text = text.strip() if text else None
        if text is None or not is_content(text):
            continue
        identifier = row.get("uuid") or hash_text(text)
        if identifier in seen:
            continue
        seen.add(identifier)
        turns.append(dict(text=MASK.mask_text(redact(text)), line=number))
    meta = dict(
        source_id=hash_text(str(path)),
        path=str(path),
        sha256=checksum.hexdigest(),
        project_id=project_id(project),
        source=source,
        users=len(turns),
        invalid_lines=invalid,
    )
    return turns, meta


def make_case(turns: list, meta: dict, excluded: Counter) -> dict | None:
    if len(turns) < 3:
        excluded["fewer_than_2_followups"] += 1
        return None
    text = turns[0]["text"]
    if len(text) < 20 or re.match(
        r"^(계속|이어서|재개|응\b|네\b|좋아|진행해|continue\b|resume\b|이거|그거)",
        text,
        re.I,
    ):
        excluded["not_self_contained_start"] += 1
        return None
    if "target-model-choice" in text or "constraint-model-compare" in text:
        excluded["current_experiment"] += 1
        return None
    future = [t["text"] for t in turns[1:9]]
    if len(text.encode()) > 24000 or len(json.dumps(future).encode()) > 96000:
        excluded["context_size"] += 1
        return None
    return dict(
        sample_id=hash_text(meta["source_id"] + str(turns[0]["line"])),
        source_id=meta["source_id"],
        source=meta["source"],
        project_id=meta["project_id"],
        category=category(text),
        input=text,
        future=future,
        state="chat: idle\nprevious input handled as: none\nuser input: " + text,
    )


def select(cases: list) -> list[dict]:
    rng = random.Random(SEED)
    strata = defaultdict(list)
    for case in cases:
        strata[(case["category"], case["project_id"], case["source"])].append(case)
    keys = sorted(strata)
    rng.shuffle(keys)
    for key in keys:
        rng.shuffle(strata[key])
    chosen = []
    while len(chosen) < 100 and any(strata.values()):
        for key in keys:
            if strata[key] and len(chosen) < 100:
                chosen.append(strata[key].pop())
    if len(chosen) != 100:
        raise RuntimeError("fewer than 100 eligible starts")
    rng.shuffle(chosen)
    return chosen


# cost: time O(b), heap O(u), io session inventory; vars: b = bytes, u = eligible user text; basis: estimate
def main() -> None:
    setup()
    if (PRIVATE / "samples.json").exists():
        print(json.dumps(read(PRIVATE / "sampling.json")))
        return
    cases, census, excluded, seen = [], [], Counter(), set()
    sources = [
        ("claude", sorted((Path.home() / ".claude/projects").glob("*/*.jsonl"))),
        ("codex", sorted((Path.home() / ".codex/sessions").glob("**/*.jsonl"))),
    ]
    for source, paths in sources:
        for path in paths:
            if "experiment-338-target-model" in str(path):
                excluded["experiment_session"] += 1
                continue
            if source == "codex":
                with path.open("rb") as stream:
                    try:
                        metadata = json.loads(stream.readline())
                    except ValueError:
                        excluded["invalid_session_metadata"] += 1
                        continue
                origin = metadata.get("payload", {}).get("source")
                if origin not in ("cli", "vscode"):
                    excluded["noninteractive_or_subagent_session"] += 1
                    continue
            elif any(
                marker in path.parent.name
                for marker in (
                    "-runtime-",
                    "-local-experiments-",
                    "-private-tmp-",
                    "-tmp-",
                )
            ):
                excluded["automated_work_directory"] += 1
                continue
            turns, meta = extract(path, source)
            census.append(meta)
            case = make_case(turns, meta, excluded)
            if case is None:
                continue
            key = hash_text(case["input"])
            if key in seen:
                excluded["duplicate_input"] += 1
                continue
            seen.add(key)
            cases.append(case)
    chosen = select(cases)
    stats = dict(
        seed=SEED,
        files=len(census),
        extracted_users=sum(m["users"] for m in census),
        eligible=len(cases),
        excluded=dict(excluded),
        selected=len(chosen),
        projects=len({c["project_id"] for c in chosen}),
        by_source=dict(Counter(c["source"] for c in chosen)),
        by_category=dict(Counter(c["category"] for c in chosen)),
    )
    for name, value in [("census", census), ("samples", chosen), ("sampling", stats)]:
        write(PRIVATE / (name + ".json"), value)
    write(
        PRIVATE / "sample-seal.json",
        {
            name: digest(PRIVATE / name)
            for name in ["census.json", "samples.json", "sampling.json"]
        },
    )
    print(json.dumps(stats))


if __name__ == "__main__":
    main()
