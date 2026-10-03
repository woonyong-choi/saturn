"""선택 대화 전체에 두 독립 라벨러를 적용하고 응답을 private에 저장한다."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shlex
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import PRIVATE_ROOT, ensure_private_root, read_jsonl  # noqa: E402

MAX_CALLS = 40
LABELERS = {
    "codex-gpt-6-astra": os.environ.get(
        "SATURN_CODEX_LABEL_COMMAND",
        "codex exec --model gpt-6-astra --ephemeral --sandbox read-only",
    ),
    "claude-sonnet": os.environ.get(
        "SATURN_CLAUDE_LABEL_COMMAND",
        "claude -p --model sonnet --tools=",
    ),
}


def build_prompt(conversation_id: str, turns: list[dict]) -> str:
    serialized = []
    for row in turns:
        serialized.append(json.dumps({
            "turn_id": row["turn_id"],
            "turn_index": row["turn_index"],
            "text": row["text"],
        }, ensure_ascii=False))
    return f"""너는 Saturn의 실제 Claude Code 대화 제약 라벨러다. 도구를 사용하지 말고 아래 대화 전체를 읽은 뒤 JSON 하나만 반환한다.

대화 식별자: {conversation_id}

제약은 다음 세 조건을 모두 만족할 때만 true다.
1. 이번 요청 하나가 아니라 앞으로도 적용된다. `앞으로`, `항상`, `통일해`, `하지 마`가 있거나 커밋 메시지·주석처럼 범주 전체를 대상으로 하면 지속한다. `이번`, `이 파일`, `이 줄만`, `방금 만든`처럼 하나로 한정하면 지속하지 않는다.
2. 무엇을 할지가 아니라 언어, 도구, 형식, 이름 규칙, 길이, 금지, 확인 절차처럼 일을 하는 방식을 제한한다.
3. 일 요청과 섞여 있어도 지속하는 방식 지정이 하나라도 있으면 제약이다. 그 일에만 걸리면 제약이 아니다.

각 턴에 `is_constraint`를 붙인다. 제약인 턴은 `operation`을 `register`, `replace`, `partial`, `release`, `none` 중 하나로 붙인다. `replace`는 같은 대상의 값을 바꾸어 앞 제약을 지킬 수 없는 경우다. `partial`은 일부 범위만 예외로 두는 경우다. `release`는 앞 제약을 풀어 더 이상 적용하지 않는 경우다. `none`은 앞 제약과 관계가 없거나 함께 지킬 수 있는 제약이다. `targets`에는 직접 해제·대체·부분 변경한 앞 턴만 넣는다. `scope`와 `reason`은 원문을 복사하지 않는 짧은 한국어로 쓴다. 마지막에는 대화 끝에 유효한 제약 턴만 `final_active_turn_ids`에 넣는다.

반드시 다음 JSON 모양만 반환한다.
{{"conversation_id":"{conversation_id}","turns":[{{"turn_id":"u-0001","is_constraint":true,"operation":"register","targets":[],"scope":"형식","reason":"앞으로 적용되는 출력 형식"}}],"final_active_turn_ids":["u-0001"]}}

대화 전체:
""" + "\n".join(serialized)


def parse_json(text: str) -> dict:
    stripped = text.strip()
    if stripped.startswith("```"):
        stripped = stripped.strip("`")
        if stripped.startswith("json"):
            stripped = stripped[4:]
    start = stripped.find("{")
    end = stripped.rfind("}")
    if start < 0 or end <= start:
        raise ValueError("JSON 응답이 없다")
    return json.loads(stripped[start:end + 1])


def run_labeler(command: str, prompt: str) -> tuple[int, str, str]:
    result = subprocess.run(
        shlex.split(command),
        input=prompt,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode, result.stdout, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    ensure_private_root()
    selected_path = PRIVATE_ROOT / "selected.jsonl"
    if not selected_path.exists():
        raise RuntimeError("selected.jsonl이 없다: 먼저 01-collect.py를 실행한다")
    rows = read_jsonl(selected_path)
    conversations: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        conversations[row["conversation_id"]].append(row)
    prompts = {
        conversation_id: build_prompt(conversation_id, sorted(turns, key=lambda row: row["turn_index"]))
        for conversation_id, turns in conversations.items()
    }
    prompt_dir = PRIVATE_ROOT / "label_requests"
    prompt_dir.mkdir(parents=True, exist_ok=True)
    for conversation_id, prompt in prompts.items():
        (prompt_dir / f"{conversation_id}.txt").write_text(prompt, encoding="utf-8")
    if args.dry_run:
        print(f"라벨 드라이런 통과: 모델 {len(LABELERS)}개, 대화 {len(prompts)}개, 최대 호출 모델별 {len(prompts)}회")
        return 0

    label_dir = PRIVATE_ROOT / "labels"
    label_dir.mkdir(parents=True, exist_ok=True)
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    for labeler, command in LABELERS.items():
        completed = sum(
            1 for path in label_dir.glob(f"{labeler}-*.jsonl")
            for row in read_jsonl(path)
            if row.get("status") in ("ok", "invalid")
        )
        if completed + len(prompts) > MAX_CALLS:
            raise RuntimeError(f"{labeler} 라벨 호출 상한 40회에 닿아 중단한다")
        output_path = label_dir / f"{labeler}-{run_id}.jsonl"
        with output_path.open("a", encoding="utf-8", newline="\n") as output:
            for conversation_id, prompt in prompts.items():
                try:
                    return_code, stdout, _ = run_labeler(command, prompt)
                    parsed = parse_json(stdout) if return_code == 0 else None
                    status = "ok" if parsed is not None else "invalid"
                    error = None if status == "ok" else f"returncode={return_code}"
                except (OSError, ValueError, json.JSONDecodeError) as exc:
                    stdout = ""
                    parsed = None
                    status = "invalid"
                    error = type(exc).__name__
                output.write(json.dumps({
                    "labeler": labeler,
                    "conversation_id": conversation_id,
                    "status": status,
                    "error": error,
                    "raw_response": stdout,
                    "parsed": parsed,
                }, ensure_ascii=False) + "\n")
                output.flush()
        print(f"라벨 끝: {labeler} {len(prompts)}회")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
