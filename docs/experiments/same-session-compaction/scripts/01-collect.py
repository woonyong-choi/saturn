"""Collect actual Claude-session compaction branches from synthetic Read histories."""

from __future__ import annotations

import hashlib
import json
import os
import platform
import random
import subprocess
import sys
import uuid
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
LOCAL = ROOT / ".local/experiments/same-session-compaction/formal"
PACKET = ROOT / "target/debug/examples/packet"
SEED = 20261005
WORDS = "북극 나무 연필 보라 구름 고래 지붕 시계 모래 은하".split()


def cli(
    cwd: Path,
    prompt: str,
    *,
    resume: str | None = None,
    session_id: str | None = None,
    fork: bool = False,
    plugin: bool = False,
    tools: str = "",
) -> tuple[dict[str, Any], dict[str, Any]]:
    args = [
        "claude",
        "-p",
        prompt,
        "--model",
        "sonnet",
        "--tools",
        tools,
        "--setting-sources",
        "project",
        "--strict-mcp-config",
        "--output-format",
        "stream-json",
        "--include-hook-events",
        "--verbose",
    ]
    if resume:
        args += ["--resume", resume]
    if session_id:
        args += ["--session-id", session_id]
    if fork:
        args += ["--fork-session"]
    if plugin:
        args += ["--plugin-dir", "/tmp/saturn-compare-fast-jev"]
    env = os.environ.copy()
    if plugin:
        env["CLAUDE_CODE_ENABLE_FUNCTION_HOOKS"] = "1"
    else:
        env.pop("TYPESAFE_API_KEY", None)
    done = subprocess.run(
        args, cwd=cwd, env=env, text=True, capture_output=True, timeout=180
    )
    if done.returncode:
        raise RuntimeError(f"claude exit {done.returncode}: {done.stderr[:250]}")
    items = [json.loads(line) for line in done.stdout.splitlines() if line.strip()]
    results = [x for x in items if x.get("type") == "result"]
    if not results:
        raise RuntimeError(f"claude emitted no result: {done.stderr[:250]}")
    result = results[-1]
    if result.get("is_error"):
        raise RuntimeError("claude returned an error result")
    logs = [
        x.get("text", "")
        for x in items
        if x.get("type") == "system" and x.get("subtype") == "ui_log"
    ]
    reads = sum(
        1
        for x in items
        if x.get("type") == "assistant"
        for y in x.get("message", {}).get("content", [])
        if isinstance(y, dict)
        and y.get("type") == "tool_use"
        and y.get("name") == "Read"
    )
    return result, {"logs": logs, "reads": reads, "raw": done.stdout}


def code(rng: random.Random) -> str:
    alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
    return "".join(rng.choice(alphabet) for _ in range(12))


def scenario(index: int, rng: random.Random) -> tuple[Path, int, str]:
    folder = LOCAL / f"scenario-{index:03}"
    folder.mkdir(parents=True, exist_ok=True)
    values = []
    records = []
    seq = 1
    for j in range(1, 9):
        value = code(rng)
        values.append(value)
        filler = " ".join(rng.choice(WORDS) for _ in range(320))
        body = f"주문 {index:03}-{j:02} 검증 코드 {value}\n참고: {filler}\n"
        filename = f"case-{j:02}.txt"
        (folder / filename).write_text(body, encoding="utf-8")
        records.append(
            {
                "seq": seq,
                "kind": "tool",
                "tool": "Read",
                "args": {"file_path": filename},
                "result": body,
            }
        )
        seq += 1
    target = (index - 1) % 8 + 1
    (folder / "scenario.jsonl").write_text(
        json.dumps(
            {
                "scenario_id": f"scenario-{index:03}",
                "record": records,
            },
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
    )
    return folder, target, values[target - 1]


def packet_for(folder: Path, index: int) -> dict[str, Any]:
    args = [
        str(PACKET),
        "--budget-tokens",
        "2048",
        "--condition",
        "rrf-only",
        "--scenarios",
        str(folder / "scenario.jsonl"),
        "--scenario-id",
        f"scenario-{index:03}",
        "--format",
        "json",
    ]
    done = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, timeout=60)
    if done.returncode:
        raise RuntimeError(f"packet exit {done.returncode}: {done.stderr[:250]}")
    return json.loads(done.stdout)


def collect_one(index: int, rng: random.Random) -> dict[str, Any]:
    folder, target, answer = scenario(index, rng)
    sid = str(uuid.uuid4())
    costs = {}
    raw = {}
    read_count = 0
    for turn in range(1, 5):
        first, second = 2 * turn - 1, 2 * turn
        intro = (
            (
                "나중에 여덟 주문 중 어느 것의 검증 코드를 물을지 모릅니다. "
                "각 파일을 Read 도구로 읽되 지금은 코드 대신 주문 번호만 답하세요. "
            )
            if turn == 1
            else ""
        )
        prompt = (
            intro
            + f"case-{first:02}.txt와 case-{second:02}.txt를 Read로 읽고 두 주문 번호만 답하세요."
        )
        result, info = cli(
            folder,
            prompt,
            session_id=sid if turn == 1 else None,
            resume=sid if turn != 1 else None,
            tools="Read",
        )
        read_count += info["reads"]
        costs[f"base_{turn}"] = result.get("total_cost_usd")
        raw[f"base_{turn}"] = info["raw"]
    if read_count < 8:
        raise RuntimeError(f"initial Read calls {read_count}/8")
    question = (
        f"앞에서 읽은 주문 {index:03}-{target:02}의 검증 코드 12자리를 "
        "그대로 한 개만 답하세요. 파일을 다시 읽지 마세요. 모르면 모른다고 답하세요."
    )
    arms = {}
    order = ["fast", "native"] if index % 2 else ["native", "fast"]
    for arm in order:
        result, info = cli(
            folder, "/compact", resume=sid, fork=True, plugin=arm == "fast"
        )
        arm_sid = result.get("session_id")
        if not arm_sid or arm_sid == sid:
            raise RuntimeError(f"{arm} fork failed")
        raw[f"{arm}_compact"] = info["raw"]
        costs[f"{arm}_compact"] = result.get("total_cost_usd")
        answer_result, answer_info = cli(folder, question, resume=arm_sid)
        raw[f"{arm}_answer"] = answer_info["raw"]
        costs[f"{arm}_answer"] = answer_result.get("total_cost_usd")
        logs = info["logs"]
        arms[arm] = {
            "session_id": arm_sid,
            "text": answer_result.get("result", ""),
            "exact": answer in answer_result.get("result", ""),
            "hook_applied": any("no summary" in x for x in logs)
            if arm == "fast"
            else None,
            "logs": logs,
        }
    packet = packet_for(folder, index)
    packet_prompt = packet["packet"] + "\n\n" + question
    packet_result, packet_info = cli(
        folder, packet_prompt, session_id=str(uuid.uuid4())
    )
    raw["packet_answer"] = packet_info["raw"]
    costs["packet_answer"] = packet_result.get("total_cost_usd")
    arms["packet"] = {
        "session_id": packet_result.get("session_id"),
        "text": packet_result.get("result", ""),
        "exact": answer in packet_result.get("result", ""),
        "packet_tokens": packet.get("tokens"),
        "included": packet.get("included"),
    }
    full_result, full_info = cli(folder, question, resume=sid)
    raw["full_answer"] = full_info["raw"]
    costs["full_answer"] = full_result.get("total_cost_usd")
    arms["full"] = {
        "session_id": sid,
        "text": full_result.get("result", ""),
        "exact": answer in full_result.get("result", ""),
    }
    for name, contents in raw.items():
        (folder / f"{name}.jsonl").write_text(contents, encoding="utf-8")
    (folder / "packet.json").write_text(
        json.dumps(packet, ensure_ascii=False), encoding="utf-8"
    )
    return {
        "index": index,
        "target": target,
        "answer": answer,
        "base_session": sid,
        "reads": read_count,
        "arms": arms,
        "costs": costs,
    }


def main() -> None:
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 48
    if not 1 <= n <= 48:
        raise SystemExit("scenario count must be 1..48")
    if not os.environ.get("TYPESAFE_API_KEY"):
        raise SystemExit("TYPESAFE_API_KEY missing; provide it through hidden stdin")
    LOCAL.mkdir(parents=True, exist_ok=True)
    raw_path = LOCAL / "results.jsonl"
    if raw_path.exists():
        raise SystemExit("results.jsonl exists; refuse to overwrite raw data")
    env_path = ROOT / "docs/experiments/same-session-compaction/env.json"
    env = {
        "date_utc": subprocess.check_output(
            ["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], text=True
        ).strip(),
        "os": platform.platform(),
        "cpu": platform.processor(),
        "claude_version": subprocess.check_output(
            ["claude", "--version"], text=True
        ).strip(),
        "model_requested": "sonnet",
        "saturn_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "fast_jev_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd="/tmp/saturn-compare-fast-jev", text=True
        ).strip(),
        "seed": SEED,
    }
    env_path.write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n")
    if not PACKET.exists():
        subprocess.run(
            ["cargo", "build", "-q", "-p", "saturn-core", "--example", "packet"],
            cwd=ROOT,
            check=True,
        )
    rng = random.Random(SEED)
    consecutive = 0
    with raw_path.open("w", encoding="utf-8") as output:
        for index in range(1, n + 1):
            try:
                result = collect_one(index, rng)
                consecutive = 0
            except Exception as error:
                result = {"index": index, "error": str(error)}
                consecutive += 1
            output.write(json.dumps(result, ensure_ascii=False) + "\n")
            output.flush()
            print(
                f"{index}/{n} "
                + ("ok" if "error" not in result else f"error: {result['error']}"),
                flush=True,
            )
            if consecutive >= 3:
                break
    data = raw_path.read_bytes()
    print("raw_sha256", hashlib.sha256(data).hexdigest())


if __name__ == "__main__":
    main()
