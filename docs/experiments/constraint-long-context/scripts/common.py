"""실험 스크립트가 공유하는 private 저장소, 가림, 토큰화 함수."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import unicodedata
from pathlib import Path

PUBLIC_ROOT = Path(__file__).resolve().parent.parent
PRIVATE_ROOT = Path(os.environ.get(
    "SATURN_EXPERIMENT_ROOT",
    "/Users/woonyong/workspace/oss/saturn/.local/experiments/constraint-long-context",
))
MASK_TOKEN = "[secret]"
EMAIL_TOKEN = "[email]"
PATH_TOKEN = "[abs]"

AUTO_MARKERS = (
    "<system-reminder>",
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-caveat>",
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<task-notification>",
    "<available-deferred-tools>",
)

SECRET_PATTERNS = (
    re.compile(r"\b(?:sk-ant-api|sk-proj|sk)-[A-Za-z0-9_-]{16,}\b"),
    re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}\b"),
    re.compile(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b"),
    re.compile(r"\b(?:xoxb|xoxp|xoxa|xoxr)-[A-Za-z0-9-]{16,}\b"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    re.compile(r"\bAIza[0-9A-Za-z_-]{20,}\b"),
    re.compile(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{16,}"),
    re.compile(r"(?i)\b(?:api[_-]?key|token|secret|password)\s*[:=]\s*[^\s,;]+"),
)
EMAIL_PATTERN = re.compile(r"\b[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b")
ABS_PATH_PATTERN = re.compile(
    r"(?<![A-Za-z0-9_])/(?:Users|home|private|tmp|var|opt|Applications|Volumes|workspace|srv|etc)/[^\s\"'<>`()]+"
)


def ensure_private_root() -> None:
    """private 경로가 Git에서 제외됐는지 확인하고 만든다."""
    result = subprocess.run(
        ["git", "check-ignore", "-q", ".local/experiments/constraint-long-context/.private"],
        cwd=PUBLIC_ROOT.parents[2],
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(".local이 Git ignore 대상이 아니어서 private 자료를 저장하지 않는다")
    PRIVATE_ROOT.mkdir(parents=True, exist_ok=True)


def mask_text(text: str) -> str:
    """Jev state에 넣기 전 비밀값, 이메일, 절대 경로를 가린다."""
    masked = text
    for pattern in SECRET_PATTERNS:
        masked = pattern.sub(MASK_TOKEN, masked)
    masked = EMAIL_PATTERN.sub(EMAIL_TOKEN, masked)

    def replace_path(match: re.Match[str]) -> str:
        path = match.group(0).rstrip(".,;:)]}")
        suffix = Path(path).name or "path"
        return f"{PATH_TOKEN}/{suffix}"

    return ABS_PATH_PATTERN.sub(replace_path, masked)


def extract_paths(text: str) -> list[str]:
    """입력의 절대 경로를 끝 이름만 남긴 경로로 반환한다."""
    paths = []
    for match in ABS_PATH_PATTERN.finditer(text):
        path = match.group(0).rstrip(".,;:)]}")
        paths.append(f"{PATH_TOKEN}/{Path(path).name or 'path'}")
    return sorted(set(paths))


def has_auto_marker(text: str) -> bool:
    return any(marker in text for marker in AUTO_MARKERS)


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as file:
        for row in rows:
            file.write(json.dumps(row, ensure_ascii=False) + "\n")


def hangul_bigrams(text: str) -> set[str]:
    normalized = unicodedata.normalize("NFC", text)
    chunks: list[str] = []
    current = ""
    for char in normalized:
        if "가" <= char <= "힣":
            current += char
        else:
            if current:
                chunks.extend(current[i:i + 2] for i in range(max(1, len(current) - 1)))
                current = ""
    if current:
        chunks.extend(current[i:i + 2] for i in range(max(1, len(current) - 1)))
    return set(chunks)


def identifier_tokens(text: str) -> set[str]:
    normalized = unicodedata.normalize("NFC", text)
    return {token.lower() for token in re.findall(r"[A-Za-z]+|\d+", normalized)}


def fragments(text: str) -> set[str]:
    return hangul_bigrams(text) | identifier_tokens(text)


def overlap_score(latest: dict, candidate: dict) -> tuple[int, int, int]:
    latest_fragments = fragments(latest.get("text", ""))
    candidate_fragments = fragments(candidate.get("text", ""))
    word_overlap = len(latest_fragments & candidate_fragments)
    file_overlap = len(set(latest.get("files", [])) & set(candidate.get("files", [])))
    return (file_overlap, word_overlap, int(candidate.get("turn_index", 0)))


def top_candidates(latest: dict, active: list[dict], limit: int = 10) -> list[dict]:
    return sorted(active, key=lambda row: overlap_score(latest, row), reverse=True)[:limit]


def get_text(row: dict) -> str:
    message = row.get("message") if isinstance(row.get("message"), dict) else row
    content = message.get("content", "") if isinstance(message, dict) else ""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(
            block.get("text", "") for block in content
            if isinstance(block, dict) and block.get("type") == "text" and isinstance(block.get("text"), str)
        )
    return ""


def is_human_user_row(row: dict) -> bool:
    if row.get("type") != "user":
        return False
    message = row.get("message") if isinstance(row.get("message"), dict) else {}
    if message.get("role") not in (None, "user"):
        return False
    if row.get("isMeta") is True or row.get("isSidechain") is True:
        return False
    text = get_text(row).strip()
    return bool(text) and not has_auto_marker(text)
