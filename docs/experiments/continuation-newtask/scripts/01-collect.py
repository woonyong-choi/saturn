"""출처별 무작위 대조 표본과 결정적 새 작업 후보를 추출한다."""

from __future__ import annotations

import hashlib
import json
import random
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

from runtime import (
    BASE,
    PRIVATE,
    PUBLIC,
    SEED,
    WORKTREE,
    extractor,
    mask,
    now,
    read,
    setup,
    sha,
    write,
)

AUTO_PREFIXES = (
    "# AGENTS.md instructions",
    "<environment_context>",
    "<permissions instructions>",
    "<user_instructions>",
    "<INSTRUCTIONS>",
    "<turn_aborted>",
    "<subagent_notification>",
    "<skill",
    "You are a helpful assistant",
    "We need produce a concise",
    "This session is being continued",
    "You have been",
)


def is_injected(text: str) -> bool:
    return text.startswith(AUTO_PREFIXES) or extractor.previous.has_auto_marker(text)


def codex_messages(path: Path, counts: Counter) -> tuple[list[dict], dict]:
    data = path.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    meta, messages = {}, []
    for line_no, line in enumerate(data.splitlines(), 1):
        try:
            row = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            counts["invalid_json"] += 1
            continue
        payload = row.get("payload", {})
        if row.get("type") == "session_meta":
            meta = payload
        if row.get("type") == "compacted":
            messages.append({"reset": True})
        if row.get("type") == "event_msg" and payload.get("type") == "task_complete":
            messages.append({"complete": True})
        if row.get("type") != "response_item" or payload.get("type") != "message":
            continue
        role = payload.get("role")
        if role not in ("user", "assistant"):
            continue
        text = "\n".join(
            b.get("text", "")
            for b in payload.get("content", [])
            if b.get("type") in ("input_text", "output_text", "text")
        ).strip()
        if not text:
            counts["empty_text"] += 1
            continue
        if role == "user" and is_injected(text):
            counts["injected_user"] += 1
            continue
        messages.append(
            {
                "role": role,
                "text": mask(text),
                "line": line_no,
                "timestamp": row.get("timestamp"),
                "phase": payload.get("phase"),
                "message_id": payload.get("id"),
            }
        )
    info = {"source_path": str(path), "sha256": digest, "bytes": len(data)}
    source = meta.get("source")
    if (
        meta.get("parent_thread_id")
        or isinstance(source, dict)
        or meta.get("thread_source") == "subagent"
    ):
        counts["subagent_files"] += 1
        return [], info
    if sha(path) != digest:
        counts["changed_files"] += 1
        return [], info
    project = str(meta.get("cwd", "unknown"))
    info["project"] = hashlib.sha256(project.encode()).hexdigest()[:16]
    return messages, info


def codex_cases(path: Path, counts: Counter) -> tuple[list[dict], dict]:
    messages, info = codex_messages(path, counts)
    prior, texts, complete, cases = None, [], False, []
    digest = extractor.digest
    for msg in messages:
        if msg.get("reset"):
            prior, texts, complete = None, [], False
            counts["compaction_reset"] += 1
            continue
        if msg.get("complete"):
            complete = True
            continue
        if msg["role"] == "assistant":
            texts.append(msg["text"])
            complete = msg["phase"] == "final"
            continue
        text = msg["text"]
        counts["human_turns"] += 1
        if prior is None:
            counts["first_turn"] += 1
        elif not re.search("[가-힣]", text):
            counts["non_korean"] += 1
        else:
            progress = "\n".join(texts[-3:])[-6000:]
            cases.append(
                {
                    "id": digest(str(path) + ":" + str(msg["line"])),
                    "source": "Codex",
                    "project": info["project"],
                    "session": digest(str(path)),
                    "source_line": msg["line"],
                    "uuid": msg["message_id"],
                    "timestamp": msg["timestamp"],
                    "input": text,
                    "previous_input": prior,
                    "goal_excerpt": prior[:2000],
                    "progress_excerpt": progress,
                    "goal_omitted_chars": max(0, len(prior) - 2000),
                    "progress_omitted_chars": max(
                        0, len("\n".join(texts)) - len(progress)
                    ),
                    "running": not complete,
                    "stop_reason": "final" if complete else None,
                    "assistant_count": len(texts),
                    "previous_disposition": "queue",
                }
            )
        prior, texts, complete = text, [], False
    return cases, info


def is_candidate(case: dict) -> bool:
    if case["running"] or len(case["input"]) < 30:
        return False
    fragments = extractor.previous.fragments
    a, b = fragments(case["input"]), fragments(case["previous_input"])
    union = a | b
    overlap = len(a & b) / len(union) if union else 1.0
    explicit = re.search(
        "별도|다른 주제|새(?:로운)? 작업|다음 작업|이제는|이번에는", case["input"]
    )
    return overlap <= 0.12 or bool(explicit)


def proportional(cases: list[dict], target: int, rng: random.Random) -> list[dict]:
    groups = defaultdict(list)
    for case in cases:
        groups[case["source"]].append(case)
    target = min(target, len(cases))
    if not target:
        return []
    quotas = {s: int(target * len(g) / len(cases)) for s, g in groups.items()}
    while sum(quotas.values()) < target:
        source = max(
            groups, key=lambda s: (target * len(groups[s]) / len(cases) - quotas[s], s)
        )
        quotas[source] += 1
    selected = []
    for source in sorted(groups):
        rng.shuffle(groups[source])
        selected.extend(groups[source][: quotas[source]])
    rng.shuffle(selected)
    return selected


def prepare() -> None:
    if (PRIVATE / "sample.json").exists():
        print("using frozen sample")
        return
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain", "--", str(PUBLIC)], cwd=WORKTREE, text=True
    )
    if dirty.strip():
        raise RuntimeError("commit design and collector before collection")
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=WORKTREE, text=True
    ).strip()
    old = read(BASE / "sample.json")
    seen_ids = {c["id"] for c in old}
    seen_uuid = {(c["project"], c["uuid"]) for c in old if c["uuid"]}
    fingerprints = {
        hashlib.sha256((c["previous_input"] + "\0" + c["input"]).encode()).hexdigest()
        for c in old
    }
    counts, population, files = {}, [], []
    for source, paths, extract in (
        (
            "Claude",
            sorted((Path.home() / ".claude/projects").glob("*/*.jsonl")),
            extractor.extract,
        ),
        (
            "Codex",
            sorted((Path.home() / ".codex/sessions").rglob("*.jsonl")),
            codex_cases,
        ),
    ):
        tally = Counter()
        for path in paths:
            cases, meta = extract(path, tally)
            files.append({**meta, "source": source})
            for case in cases:
                case["source"] = source
                key = (case["project"], case["uuid"])
                if case["id"] in seen_ids or (case["uuid"] and key in seen_uuid):
                    tally["previous_or_uuid_duplicate"] += 1
                    continue
                # 앞 실험 추가 한 턴의 원본이 없어 당시 수집일 Claude 턴까지 보수적으로 제외한다.
                if (
                    source == "Claude"
                    and str(case.get("timestamp") or "")[:10] <= "2026-10-04"
                ):
                    tally["prior_extension_overlap_guard"] += 1
                    continue
                fp = hashlib.sha256(
                    (case["previous_input"] + "\0" + case["input"]).encode()
                ).hexdigest()
                if fp in fingerprints:
                    tally["duplicate_pair"] += 1
                    continue
                fingerprints.add(fp)
                seen_ids.add(case["id"])
                if case["uuid"]:
                    seen_uuid.add(key)
                case["prefilter"] = is_candidate(case)
                population.append(case)
        counts[source] = dict(tally)
    rng = random.Random(SEED)
    random_sample = proportional(population, 800, rng)
    chosen = {c["id"] for c in random_sample}
    enriched = proportional(
        [c for c in population if c["prefilter"] and c["id"] not in chosen], 1200, rng
    )
    for c in random_sample:
        c["arm"] = "random"
    for c in enriched:
        c["arm"] = "enriched"
    sample = random_sample + enriched
    rng.shuffle(sample)
    run = {
        "run_id": now().replace(":", "").replace("-", "")[:15] + "-" + head[:7],
        "design_commit": head,
        "started": now(),
        "base_hashes": {n: sha(BASE / n) for n in ("sample.json", "final-labels.json")},
        "prior_summary_sha256": sha(
            PUBLIC.parent / "continuation-misjoin/results/summary.json"
        ),
    }
    write(PRIVATE / "run.json", run)
    write(PRIVATE / "census.json", {"files": files, "counts": counts})
    write(PRIVATE / "sample.json", sample)
    selection = {
        "population": len(population),
        "selected": len(sample),
        "counts": counts,
        "population_strata": dict(
            Counter(c["source"] + "/" + str(c["prefilter"]) for c in population)
        ),
        "sample_strata": dict(
            Counter(
                c["source"] + "/" + c["arm"] + "/" + str(c["prefilter"]) for c in sample
            )
        ),
        "files": len(files),
    }
    write(PRIVATE / "selection.json", selection)
    print(json.dumps(selection))


if __name__ == "__main__":
    setup()
    prepare()
