"""합성 작업을 Claude Code로 실행해 압축 요약을 읽고, 두 패킷으로 Codex에 질문해 raw/에 기록한다.

사용: python3 scripts/01-collect.py (실험 폴더에서 실행)

실행 조건이 갖춰지지 않으면 원인 한 줄을 표준 오류로 내고 종료 코드 2로 끝난다.
`SATURN_PACKET_CMD`는 #126 `build_packet`을 부르는 명령이다. 표준 입력으로 기록 JSONL
(`{"record_no", "event"}`)을 받고, 인자 `--mode saturn|provider --budget-tokens N
[--summary-file 파일 --summary-record 번호]`를 받아 패킷 글을 표준 출력으로 낸다.
요약이 경쟁 구역 예산을 넘어 원문으로 채웠으면 표준 오류에 `summary_fallback`을 낸다.
"""
import json
import os
import platform
import queue
import random
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

CLAUDE_VERSION = "2.1.285"
CODEX_VERSION = "0.158.0"
SEED = 122
SCENARIOS = 40
SCENARIOS_MAX = 60
COMPACTED_MIN = 30
AUTOCOMPACT = 100000
PACKET_BUDGET_TOKENS = AUTOCOMPACT // 10
TURN_TIMEOUT_SECONDS = 600
LOG_LINES = 1500
EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = EXPERIMENT_DIR / "data" / "raw"
WORK_ROOT = Path.home() / "workspace/woon/.local/orchestration/saturn-experiments/claude-summary-handoff/work"
LABELS = ["ledger_checksum", "batch_token", "replay_marker", "audit_seq", "shard_key", "settle_code", "fence_id", "drain_mark"]
SERVICES = ["billing", "payout", "refund", "invoice", "settlement"]


def now_utc() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def command_output(args: list[str]) -> str:
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def not_ready_reason() -> str | None:
    if not shutil.which("claude"):
        return "claude 실행 파일이 없다"
    if CLAUDE_VERSION not in command_output(["claude", "--version"]):
        return f"Claude Code 버전이 {CLAUDE_VERSION}이 아니다"
    if not shutil.which("codex"):
        return "codex 실행 파일이 없다"
    if CODEX_VERSION not in command_output(["codex", "--version"]):
        return f"codex-cli 버전이 {CODEX_VERSION}이 아니다"
    if not os.environ.get("SATURN_PACKET_CMD"):
        return "#126 build_packet 실행 명령 SATURN_PACKET_CMD가 없다"
    return None


def make_scenario(rng: random.Random, index: int) -> dict:
    service = rng.choice(SERVICES)
    labels = rng.sample(LABELS, len(LABELS))
    files = [
        {"path": f"logs/{service}-{k + 1:02d}.log", "label": labels[k], "value": f"{rng.randrange(100000, 1000000)}"}
        for k in range(len(LABELS))
    ]
    retry_old, retry_new = rng.sample(range(2, 10), 2)
    scenario = {
        "scenario_id": f"s{index:02d}",
        "seed": rng.randrange(1 << 30),
        "service": service,
        "port": rng.randrange(7000, 9000),
        "log_dir": f"var/{service}-logs",
        "retry_old": retry_old,
        "retry_new": retry_new,
        "files": files,
    }
    scenario["turns"] = make_turns(scenario)
    scenario["questions"] = make_questions(scenario)
    return scenario


def make_turns(s: dict) -> list[str]:
    turns = [
        f"이 저장소에서 {s['service']} 정산 기능을 만든다. 제약: API 포트는 {s['port']}을 쓴다. "
        f"로그는 {s['log_dir']}에만 쓴다. 재시도는 {s['retry_old']}회로 제한한다. 지금은 확인만 짧게 답해라."
    ]
    for k, f in enumerate(s["files"]):
        prefix = f"제약 변경: 재시도는 {s['retry_new']}회로 바꾼다. " if k == 4 else ""
        turns.append(f"{prefix}`{f['path']}` 전체를 cat으로 읽고 `{f['label']}=` 뒤의 값과 파일 경로를 한 줄로 답해라.")
    turns += [
        "README.md에 이 기능의 목적을 한 줄로 써라.",
        "src/config.txt에 현재 제약 값을 key=value 형식으로 써라.",
        "남은 할 일을 세 줄 이내로 정리해라.",
    ]
    return turns


def make_questions(s: dict) -> list[dict]:
    files = s["files"]
    questions = [
        {"question_id": f"q{k + 1:02d}", "kind": "value", "text": f"{f['label']} 값은 무엇인가", "expected": f["value"]}
        for k, f in enumerate(files[:7])
    ]
    for k, f in ((8, files[7]), (9, files[2])):
        questions.append({"question_id": f"q{k:02d}", "kind": "path", "text": f"{f['label']} 값을 담은 파일 경로는 무엇인가", "expected": f["path"]})
    questions.append({"question_id": "q10", "kind": "constraint", "text": "지금 재시도 횟수 제한은 몇 회인가", "expected": str(s["retry_new"])})
    return questions


def write_workspace(s: dict, workdir: Path) -> None:
    rng = random.Random(s["seed"])
    (workdir / "src").mkdir(parents=True, exist_ok=True)
    for f in s["files"]:
        path = workdir / f["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        planted = rng.randrange(LOG_LINES)
        lines = []
        for n in range(LOG_LINES):
            if n == planted:
                lines.append(f"{f['label']}={f['value']}")
            else:
                lines.append(f"ts={n:05d} level=info req={rng.randrange(1 << 32):08x} msg=batch step ok shard={rng.randrange(64)}")
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")


class ClaudeSession:
    def __init__(self, workdir: Path, session_id: str):
        args = [
            "claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--session-id", session_id, "--autocompact", str(AUTOCOMPACT), "--permission-mode", "acceptEdits",
        ]
        self.process = subprocess.Popen(args, cwd=workdir, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.lines: queue.Queue[str | None] = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def turn(self, text: str) -> list[dict]:
        message = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}
        self.process.stdin.write(json.dumps(message, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        events = []
        deadline = time.monotonic() + TURN_TIMEOUT_SECONDS
        while True:
            line = self.lines.get(timeout=max(0.0, deadline - time.monotonic()))
            if line is None:
                raise RuntimeError("Claude 프로세스가 턴 중에 끝났다")
            if not line.strip():
                continue
            event = json.loads(line)
            events.append(event)
            if event.get("type") == "result":
                return events

    def close(self) -> None:
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()


def local_session_file(workdir: Path, session_id: str) -> Path:
    encoded = re.sub(r"[^A-Za-z0-9]", "-", str(workdir.resolve()))
    return Path.home() / ".claude" / "projects" / encoded / f"{session_id}.jsonl"


def message_text(entry: dict) -> str:
    content = entry.get("message", {}).get("content", "")
    if isinstance(content, str):
        return content
    return "".join(part.get("text", "") for part in content if isinstance(part, dict))


def local_compactions(path: Path) -> list[dict]:
    if not path.exists():
        return []
    entries = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    found = []
    for i, entry in enumerate(entries):
        if entry.get("type") == "system" and entry.get("subtype") == "compact_boundary":
            summary = next((message_text(e) for e in entries[i + 1:] if e.get("isCompactSummary")), "")
            trigger = entry.get("compactMetadata", {}).get("trigger")
            found.append({"uuid": entry.get("uuid"), "trigger": trigger, "summary": summary})
    return found


def run_claude(s: dict, run_id: str, workdir: Path) -> tuple[list[dict], list[dict], str | None]:
    session_id = str(uuid.uuid4())
    records: list[dict] = []
    error = None
    session = ClaudeSession(workdir, session_id)
    try:
        for turn_no, text in enumerate(s["turns"], start=1):
            records.append({"record_no": len(records) + 1, "turn": turn_no, "event": {"type": "saturn_user_input", "text": text}})
            for event in session.turn(text):
                records.append({"record_no": len(records) + 1, "turn": turn_no, "event": event})
    except (RuntimeError, queue.Empty, json.JSONDecodeError) as failure:
        error = f"{type(failure).__name__}: {failure}"
    finally:
        session.close()
    stream_boundaries: dict[str, list[int]] = {}
    stream_triggers: dict[str, str | None] = {}
    for record in records:
        event = record["event"]
        if event.get("type") == "system" and event.get("subtype") == "compact_boundary":
            stream_boundaries.setdefault(event.get("uuid"), []).append(record["record_no"])
            stream_triggers[event.get("uuid")] = event.get("compact_metadata", {}).get("trigger")
    locals_found = local_compactions(local_session_file(workdir, session_id))
    local_uuids = {local["uuid"] for local in locals_found}
    missing = [
        {"uuid": u, "trigger": stream_triggers[u], "summary": "", "missing": True}
        for u in stream_boundaries if u not in local_uuids
    ]
    rows = []
    for index, local in enumerate(locals_found + missing, start=1):
        matches = stream_boundaries.get(local["uuid"], [])
        rows.append({
            "run_id": run_id,
            "trial_id": f"{s['scenario_id']}-c{index}",
            "condition": "claude-read",
            "ts_utc": now_utc(),
            "scenario_id": s["scenario_id"],
            "kind": "compaction",
            "compaction_index": index,
            "trigger": local["trigger"],
            "local_boundary_found": not local.get("missing", False),
            "summary_text": local["summary"],
            "stream_match_count": len(matches),
            "stream_boundary_record_no": matches[0] if len(matches) == 1 else None,
        })
    return rows, records, error


def build_packet(records: list[dict], mode: str, summary: dict | None, summary_path: Path) -> tuple[str, bool]:
    args = shlex.split(os.environ["SATURN_PACKET_CMD"]) + ["--mode", mode, "--budget-tokens", str(PACKET_BUDGET_TOKENS)]
    if summary is not None:
        summary_path.write_text(summary["summary_text"], encoding="utf-8")
        args += ["--summary-file", str(summary_path), "--summary-record", str(summary["stream_boundary_record_no"] - 1)]
    stdin = "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records)
    done = subprocess.run(args, input=stdin, capture_output=True, text=True, check=True)
    return done.stdout, "summary_fallback" in done.stderr


def ask_codex(packet: str, questions: list[dict], empty_dir: Path) -> tuple[dict, bool, str | None]:
    lines = "\n".join(f"{q['question_id']}: {q['text']}" for q in questions)
    prompt = (
        f"{packet}\n\n위 맥락만 보고 답해라. 도구나 명령을 쓰지 마라. 모르면 \"모름\"이라고 답해라.\n"
        f"답은 JSON 객체 하나로, 키는 질문 id, 값은 답 문자열이다.\n{lines}\n"
    )
    args = ["codex", "exec", "--json", "--sandbox", "read-only", "--skip-git-repo-check", "-C", str(empty_dir), "-"]
    done = subprocess.run(args, input=prompt, capture_output=True, text=True, timeout=TURN_TIMEOUT_SECONDS)
    tool_used = False
    message = ""
    for line in done.stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item", {})
        if item.get("type") in ("command_execution", "file_change", "mcp_tool_call", "web_search"):
            tool_used = True
        if event.get("type") == "item.completed" and item.get("type") == "agent_message":
            message = item.get("text", "")
    if done.returncode != 0:
        return {}, tool_used, f"codex 종료 코드 {done.returncode}"
    match = re.search(r"\{.*\}", message, re.DOTALL)
    try:
        return json.loads(match.group(0)) if match else {}, tool_used, None if match else "답에 JSON 없음"
    except json.JSONDecodeError:
        return {}, tool_used, "답 JSON 해석 실패"


def write_rows(path: Path, rows: list[dict]) -> None:
    with path.open("a", encoding="utf-8", newline="\n") as out:
        for row in rows:
            out.write(json.dumps(row, ensure_ascii=False) + "\n")


def write_env(run_id: str, commit: str) -> None:
    env = {
        "os": f"{platform.system()} {platform.release()}",
        "cpu": command_output(["sysctl", "-n", "machdep.cpu.brand_string"]) or platform.processor(),
        "memory_bytes": int(command_output(["sysctl", "-n", "hw.memsize"]) or 0) or None,
        "tools": {
            "python": platform.python_version(),
            "claude": command_output(["claude", "--version"]),
            "codex": command_output(["codex", "--version"]),
            "saturn_packet_cmd": os.environ["SATURN_PACKET_CMD"],
        },
        "models": {
            "claude": os.environ.get("ANTHROPIC_MODEL", "Claude Code 기본 모델"),
            "codex": os.environ.get("CODEX_MODEL", "codex-cli 기본 모델"),
        },
        "run_date": now_utc()[:10],
        "run_id": run_id,
        "commit": commit,
        "seed": SEED,
        "autocompact": AUTOCOMPACT,
        "packet_budget_tokens": PACKET_BUDGET_TOKENS,
    }
    (EXPERIMENT_DIR / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    reason = not_ready_reason()
    if reason:
        print(f"수집 불가: {reason}", file=sys.stderr)
        return 2
    commit = command_output(["git", "rev-parse", "--short=7", "HEAD"])
    run_id = f"{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{commit}"
    RAW_DIR.mkdir(parents=True, exist_ok=True)
    claude_raw, packet_raw, codex_raw = (RAW_DIR / f"{name}-{run_id}.jsonl" for name in ("claude", "packet", "codex"))
    work = WORK_ROOT / run_id
    empty_dir = work / "codex-empty"
    empty_dir.mkdir(parents=True, exist_ok=True)
    write_env(run_id, commit)
    rng = random.Random(SEED)
    compacted = 0
    index = 0
    while index < SCENARIOS or (compacted < COMPACTED_MIN and index < SCENARIOS_MAX):
        index += 1
        s = make_scenario(rng, index)
        workdir = work / s["scenario_id"]
        write_workspace(s, workdir)
        compactions, records, error = run_claude(s, run_id, workdir)
        auto = compactions
        compacted += 1 if auto else 0
        write_rows(claude_raw, compactions + [{
            "run_id": run_id, "trial_id": s["scenario_id"], "condition": "claude-read", "ts_utc": now_utc(),
            "scenario_id": s["scenario_id"], "kind": "scenario", "turns": len(s["turns"]),
            "compaction_count": len(auto), "error": error,
        }])
        usable = [c for c in auto if c["local_boundary_found"] and c["summary_text"] and c["stream_boundary_record_no"] is not None]
        if error or not auto or len(usable) != len(auto):
            continue
        conditions = ["claude-summary", "saturn-packet"]
        rng.shuffle(conditions)
        for condition in conditions:
            summary = usable[-1] if condition == "claude-summary" else None
            mode = "provider" if summary else "saturn"
            packet, fallback = build_packet(records, mode, summary, workdir / "summary.txt")
            write_rows(packet_raw, [{
                "run_id": run_id, "trial_id": f"{s['scenario_id']}-{condition}", "condition": condition,
                "ts_utc": now_utc(), "scenario_id": s["scenario_id"], "packet": packet, "summary_fallback": fallback,
            }])
            answers, tool_used, codex_error = ask_codex(packet, s["questions"], empty_dir)
            write_rows(codex_raw, [{
                "run_id": run_id, "trial_id": f"{s['scenario_id']}-{q['question_id']}", "condition": condition,
                "ts_utc": now_utc(), "scenario_id": s["scenario_id"], "question_id": q["question_id"],
                "question_kind": q["kind"], "expected": q["expected"], "answer": answers.get(q["question_id"]),
                "tool_used": tool_used, "summary_fallback": fallback, "packet_chars": len(packet), "error": codex_error,
            } for q in s["questions"]])
    return 0


if __name__ == "__main__":
    sys.exit(main())
