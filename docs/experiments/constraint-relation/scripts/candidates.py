"""정답을 보지 않는 후보 규칙과 바이트 제한을 고정한다."""

from __future__ import annotations

import json
import re
import unicodedata
from functools import lru_cache

from support import judge

RECENT_K = 8
MAX_BYTES = 100_000
TARGETS = {
    "commit": ("커밋", "commit"),
    "pr": ("풀 리퀘스트", "pull request", "pr"),
    "branch": ("브랜치", "branch", "worktree", "워크트리"),
    "document": ("문서", "docs", "documentation"),
    "comment": ("주석", "comment"),
    "test": ("테스트", "test", "pytest"),
    "language": ("한국어", "영어", "한글", "korean", "english"),
    "format": ("마크다운", "markdown", "표로", "표 형식"),
    "codex": ("codex", "코덱스"),
    "claude": ("claude", "클로드"),
    "rust": ("rust", "러스트"),
    "python": ("python", "파이썬"),
    "kotlin": ("kotlin", "코틀린"),
    "git": ("git", "깃"),
    "obsidian": ("obsidian", "옵시디언"),
    "wiki": ("wiki", "위키"),
    "browser": ("브라우저", "browser", "chrome"),
    "search": ("검색", "search"),
    "key": ("api 키", "api key", "키체인"),
    "agent": ("에이전트", "agent", "subagent"),
    "terminal": ("터미널", "terminal", "tui"),
    "file": ("파일", "file"),
}
REWRITES = {
    "replaces": "Does the later input set a lasting rule for the same scope that cannot coexist with the earlier rule, excluding one-time exceptions?",
    "releases": "Does the later input explicitly revoke the earlier rule for its whole scope, excluding one-time exceptions?",
}


@lru_cache(maxsize=2048)
def target_keys(text: str) -> set[str]:
    normalized = unicodedata.normalize("NFKC", text).casefold()
    keys = set()
    for key, names in TARGETS.items():
        for name in names:
            pattern = re.escape(name)
            if name.isascii():
                pattern = r"(?<![a-z0-9_])" + pattern + r"(?![a-z0-9_])"
            if re.search(pattern, normalized):
                keys.add("noun:" + key)
    paths = re.findall(
        r"[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+|[a-z0-9_-]+\.(?:md|rs|py|kt|json|yaml|toml|sh)",
        normalized,
    )
    keys.update("path:" + path for path in paths)
    return keys


def select_candidates(turn: dict, pool: list[dict], condition: str) -> list[dict]:
    if condition == "C1":
        return list(reversed(pool[-RECENT_K:]))
    keys = target_keys(turn["text"])
    return [
        candidate
        for candidate in reversed(pool)
        if keys & target_keys(candidate["text"])
    ]


def encode_request(trial: dict) -> bytes:
    return json.dumps(
        {k: trial[k] for k in ("model", "state", "questions")}, ensure_ascii=False
    ).encode()


def make_trial(turn: dict, pool: list[dict], meta: dict) -> dict:
    chosen = select_candidates(turn, pool, meta["condition"])
    before = len(chosen)
    before_ids = [c["turn_id"] for c in chosen]
    trial = {
        "model": judge.MODEL,
        "state": {"latest_user_input": turn["text"], "pairs": []},
        "questions": {},
        "meta": dict(meta),
    }
    for index, candidate in enumerate(chosen):
        trial["state"]["pairs"].append(
            {
                "pair_index": index,
                "earlier_constraint": candidate["text"],
                "later_constraint": turn["text"],
                "turn_distance": turn["turn_index"] - candidate["turn_index"],
            }
        )
        for kind, question in [
            ("replaces", judge.REPLACE_QUESTION),
            ("releases", judge.RELEASE_QUESTION),
        ]:
            wording = REWRITES[kind] if meta["condition"] == "C3" else question
            trial["questions"][f"{kind}_{index}"] = judge.question(
                f"For pair {index}: " + wording
            )
    original_bytes = len(encode_request(trial))
    size_bytes = original_bytes
    while chosen and size_bytes > MAX_BYTES:
        chosen.pop()
        pair = trial["state"]["pairs"].pop()
        size_bytes -= len(json.dumps(pair, ensure_ascii=False).encode())
        size_bytes -= 2 if chosen else 0
        for kind in ("replaces", "releases"):
            name = f"{kind}_{len(chosen)}"
            value = trial["questions"].pop(name)
            entry = json.dumps(name) + ": " + json.dumps(value, ensure_ascii=False)
            size_bytes -= len(entry.encode()) + (2 if trial["questions"] else 0)
    if size_bytes != len(encode_request(trial)):
        raise AssertionError("request byte accounting mismatch")
    trial["meta"].update(
        candidate_ids=[c["turn_id"] for c in chosen],
        before_candidate_ids=before_ids,
        before_trim=before,
        trimmed=before - len(chosen),
        original_bytes=original_bytes,
        request_bytes=len(encode_request(trial)),
        pool_size=len(pool),
    )
    trial["status"] = (
        "ready"
        if chosen
        else (
            "oversize_input"
            if len(encode_request(trial)) > MAX_BYTES
            else ("trimmed_empty" if before else "no_candidates")
        )
    )
    trial["trial_id"] = "-".join(
        meta[k] for k in ("cohort", "condition", "conversation_id", "turn_id")
    )
    return trial
