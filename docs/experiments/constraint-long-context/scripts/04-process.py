"""private Jev 응답을 분석용 정규화 행으로 만들고 공개 파일 해시를 갱신한다."""

from __future__ import annotations

import csv
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import PRIVATE_ROOT, PUBLIC_ROOT, read_jsonl, sha256_file, write_json  # noqa: E402
from _03_import import load_consensus  # noqa: E402


def answer_value(answers: dict | None, question_id: str) -> float | None:
    value = ((answers or {}).get(question_id) or {}).get("noul")
    return float(value) if isinstance(value, (int, float)) and 0 <= value <= 1 else None


def relation_for(label: dict | None, candidate_id: str) -> str | None:
    if not label or not label.get("is_constraint"):
        return None
    if candidate_id not in label.get("targets", []):
        return "compatible"
    return {
        "replace": "replace",
        "partial": "partial",
        "release": "release",
    }.get(label.get("operation"), "compatible")


def main() -> int:
    responses_path = PRIVATE_ROOT / "jev" / "responses.jsonl"
    if not responses_path.exists():
        raise RuntimeError("Jev 응답이 없다: 키가 있으면 03-jev.py를 실행한다")
    consensus = load_consensus()
    rows = read_jsonl(responses_path)
    normalized = []
    for response in rows:
        meta = response.get("meta", {})
        conversation = consensus.get(meta.get("conversation_id"), {})
        label = conversation.get("turns", {}).get(meta.get("turn_id"))
        is_answer = answer_value(response.get("answers"), "is_constraint")
        is_gold = label.get("is_constraint") if label else None
        candidate_gold_relations = {}
        if label and label.get("is_constraint"):
            for candidate_id in meta.get("candidate_ids", []):
                candidate_gold_relations[candidate_id] = (
                    {"replace": "replace", "partial": "partial", "release": "release"}.get(label.get("operation"), "compatible")
                    if candidate_id in label.get("targets", []) else "compatible"
                )
        normalized.append({
            "trial_id": response.get("trial_id"),
            "conversation_id": meta.get("conversation_id"),
            "turn_id": meta.get("turn_id"),
            "length_band": meta.get("length_band"),
            "condition": meta.get("condition"),
            "repeat": meta.get("repeat"),
            "kind": meta.get("kind", "turn"),
            "status": response.get("status"),
            "is_answer": is_answer,
            "is_predicted": is_answer is not None and is_answer >= 0.7,
            "is_gold": is_gold,
            "candidate_ids": meta.get("candidate_ids", []),
            "gold_final_ids": meta.get("gold_final_ids", []),
            "candidate_gold_relations": candidate_gold_relations,
            "answers": response.get("answers") or {},
            "latency_ms": response.get("latency_ms"),
        })
    out = PRIVATE_ROOT / "processed.jsonl"
    with out.open("w", encoding="utf-8", newline="\n") as file:
        for row in normalized:
            file.write(json.dumps(row, ensure_ascii=False) + "\n")
    env_path = PUBLIC_ROOT / "env.json"
    env = json.loads(env_path.read_text(encoding="utf-8")) if env_path.exists() else {}
    env.update({"last_processed_private_sha256": hashlib.sha256(out.read_bytes()).hexdigest(), "response_count": len(rows)})
    write_json(env_path, env)
    checksum_paths = [PUBLIC_ROOT / "design.md", PUBLIC_ROOT / "questions.json", PUBLIC_ROOT / "run.sh"]
    checksum_lines = [f"{sha256_file(path)}  {path.relative_to(PUBLIC_ROOT).as_posix()}" for path in checksum_paths if path.exists()]
    (PUBLIC_ROOT / "data" / "SHA256SUMS").write_text("\n".join(checksum_lines) + "\n", encoding="utf-8")
    print(f"처리 끝: Jev 응답 {len(rows)}건")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
