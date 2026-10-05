"""축약과 질문 사이에 Claude 프로세스를 유지하는 대응 실행을 수집한다."""

from __future__ import annotations

import json
import os
import queue
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / ".local/experiments/same-session-compaction/formal"
OUT = ROOT / ".local/experiments/same-session-compaction/persistent"


def read_events(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def drain(process: subprocess.Popen[str], lines: queue.Queue[str | None]) -> None:
    if process.stdout is None:
        raise RuntimeError("stdout unavailable")
    for line in process.stdout:
        lines.put(line)
    lines.put(None)


def turn(
    process: subprocess.Popen[str],
    lines: queue.Queue[str | None],
    prompt: str,
    path: Path,
) -> dict[str, Any]:
    if process.stdin is None:
        raise RuntimeError("stdin unavailable")
    process.stdin.write(
        json.dumps({"type": "user", "message": {"role": "user", "content": prompt}})
        + "\n"
    )
    process.stdin.flush()
    deadline = time.monotonic() + 180
    with path.open("x") as output:
        while time.monotonic() < deadline:
            line = lines.get(timeout=max(0.01, deadline - time.monotonic()))
            if line is None:
                raise RuntimeError("provider stream ended before result")
            output.write(line)
            output.flush()
            event = json.loads(line)
            if event.get("type") != "result":
                continue
            if event.get("is_error"):
                raise RuntimeError("provider returned an error result")
            return event
    raise TimeoutError("provider result deadline exceeded")


def stop(process: subprocess.Popen[str]) -> None:
    if process.stdin is not None:
        process.stdin.close()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def collect_arm(row: dict[str, Any], arm: str, root: Path) -> dict[str, Any]:
    source = SOURCE / f"scenario-{row['index']:03}"
    folder = root / source.name
    folder.mkdir(parents=True, exist_ok=True)
    base = read_events(source / "base_4.jsonl")
    parent = [e for e in base if e.get("type") == "assistant"][-1]["uuid"]
    args = [
        "claude",
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-hook-events",
        "--model",
        "sonnet",
        "--tools",
        "",
        "--setting-sources",
        "project",
        "--strict-mcp-config",
        "--resume",
        row["base_session"],
        "--resume-session-at",
        parent,
        "--fork-session",
    ]
    env = os.environ.copy()
    if arm == "fast":
        args += ["--plugin-dir", "/tmp/saturn-compare-fast-jev"]
        env["CLAUDE_CODE_ENABLE_FUNCTION_HOOKS"] = "1"
    else:
        env.pop("TYPESAFE_API_KEY", None)
    with (folder / f"{arm}_stderr.txt").open("x") as stderr:
        process = subprocess.Popen(
            args,
            cwd=source,
            env=env,
            text=True,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            start_new_session=True,
        )
        lines: queue.Queue[str | None] = queue.Queue()
        reader = threading.Thread(target=drain, args=(process, lines), daemon=True)
        reader.start()
        try:
            compact = turn(process, lines, "/compact", folder / f"{arm}_compact.jsonl")
            question = (
                f"앞에서 읽은 주문 {row['index']:03}-{row['target']:02}의 검증 코드 12자리를 "
                "그대로 한 개만 답하세요. 파일을 다시 읽지 마세요. 모르면 모른다고 답하세요."
            )
            result = turn(process, lines, question, folder / f"{arm}_answer.jsonl")
        finally:
            stop(process)
            reader.join(timeout=5)
    if compact["session_id"] != result["session_id"]:
        raise RuntimeError("session changed within persistent process")
    return {
        "session_id": result["session_id"],
        "base_parent": parent,
        "pid": process.pid,
        "text": result.get("result", ""),
        "exact": row["answer"] in result.get("result", ""),
        "cost_usd": result["total_cost_usd"] - row["costs"]["full_answer"],
        "duration_ms": compact["duration_ms"] + result["duration_ms"],
    }


def main() -> None:
    rows = read_events(SOURCE / "results.jsonl")
    if len(rows) != 48 or any("error" in row for row in rows):
        raise ValueError("formal collection must be complete")
    if "--smoke" in sys.argv:
        result = collect_arm(rows[0], "native", OUT.parent / "persistent-smoke")
        print(
            json.dumps(
                {
                    "session_id": result["session_id"],
                    "base_parent": result["base_parent"],
                }
            )
        )
        return
    if not os.environ.get("TYPESAFE_API_KEY"):
        raise ValueError("TYPESAFE_API_KEY missing")
    OUT.mkdir(parents=True, exist_ok=True)
    consecutive_errors = 0
    with (OUT / "results.jsonl").open("x") as output:
        for row in rows:
            try:
                order = ("fast", "native") if row["index"] % 2 else ("native", "fast")
                arms = {arm: collect_arm(row, arm, OUT) for arm in order}
                result = {"index": row["index"], "target": row["target"], "arms": arms}
                consecutive_errors = 0
            except Exception as error:
                result = {"index": row["index"], "error": str(error)}
                consecutive_errors += 1
            output.write(json.dumps(result, ensure_ascii=False) + "\n")
            output.flush()
            state = "error" if "error" in result else "ok"
            print(f"persistent {row['index']}/48 {state}", flush=True)
            if consecutive_errors >= 3:
                raise SystemExit("three consecutive persistent failures")


if __name__ == "__main__":
    main()
