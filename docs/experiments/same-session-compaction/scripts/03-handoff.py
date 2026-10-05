"""같은 합성 자료에서 패킷 인수와 후속 질문을 별도 턴으로 실행한다."""

from __future__ import annotations

import importlib.util
import json
import sys
import uuid
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SOURCE = ROOT / ".local/experiments/same-session-compaction/formal"
OUT = ROOT / ".local/experiments/same-session-compaction/handoff"


def load_collector() -> Any:
    spec = importlib.util.spec_from_file_location("collector", HERE / "01-collect.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("collector unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def collect_one(row: dict[str, Any], collector: Any) -> dict[str, Any]:
    source = SOURCE / f"scenario-{row['index']:03}"
    folder = OUT / source.name
    folder.mkdir(parents=True, exist_ok=True)
    packet = json.loads((source / "packet.json").read_text())
    sid = str(uuid.uuid4())
    ready, ready_info = collector.cli(
        source, packet["packet"], session_id=sid, stream=True
    )
    (folder / "packet_ready.jsonl").write_text(ready_info["raw"])
    if ready.get("is_error"):
        raise RuntimeError("packet handoff returned an error")
    question = (
        f"앞에서 읽은 주문 {row['index']:03}-{row['target']:02}의 검증 코드 12자리를 "
        "그대로 한 개만 답하세요. 파일을 다시 읽지 마세요. 모르면 모른다고 답하세요."
    )
    result, info = collector.cli(source, question, resume=sid, stream=True)
    (folder / "packet_answer.jsonl").write_text(info["raw"])
    if result.get("is_error"):
        raise RuntimeError("packet follow-up returned an error")
    return {
        "index": row["index"],
        "target": row["target"],
        "session_id": sid,
        "ready": ready.get("result", "").strip() == "Ready",
        "text": result.get("result", ""),
        "exact": row["answer"] in result.get("result", ""),
        "cost_usd": result["total_cost_usd"],
        "duration_ms": ready["duration_ms"] + result["duration_ms"],
    }


def main() -> None:
    rows = [
        json.loads(line) for line in (SOURCE / "results.jsonl").read_text().splitlines()
    ]
    if len(rows) != 48 or any("error" in row for row in rows):
        raise ValueError("formal collection must finish before handoff validation")
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / "results.jsonl"
    if path.exists():
        raise ValueError("handoff results exist; refusing to overwrite")
    collector = load_collector()
    consecutive_errors = 0
    with path.open("x") as output:
        for row in rows:
            try:
                result = collect_one(row, collector)
                consecutive_errors = 0
            except Exception as error:
                result = {"index": row["index"], "error": str(error)}
                consecutive_errors += 1
            output.write(json.dumps(result, ensure_ascii=False) + "\n")
            output.flush()
            state = "error" if "error" in result else "ok"
            print(f"handoff {row['index']}/48 {state}", flush=True)
            if consecutive_errors >= 3:
                sys.exit("three consecutive handoff failures")


if __name__ == "__main__":
    main()
