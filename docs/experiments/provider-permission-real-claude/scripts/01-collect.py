#!/usr/bin/env python3
"""Claude Code 권한 경로를 can_use_tool 요청, marker 파일, 도구 결과 이벤트로 잰다.

Saturn의 Claude 연결(`providers/claude.rs` launch_args)과 같은 stream-json 입출력, `--permission-prompt-tool stdio`,
`--settings`의 permissions.ask 목록으로 공식 `claude`를 띄우고, Saturn engine 자리에서 판정(allow/deny/ask->decline)에 답한다.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import queue
import re
import secrets
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

EXPERIMENT = Path(__file__).resolve().parent.parent
WORKTREE = EXPERIMENT.parents[2]
RUNTIME = WORKTREE / ".runtime"
MCP_FIXTURE = WORKTREE / "scripts" / "mcp_fixture.py"
DENY_HOOK = EXPERIMENT / "scripts" / "deny_hook.py"
COUNTER = RUNTIME / "claude-call-count"
CALL_LIMIT = 60
MODEL = "haiku"
TURN_TIMEOUT = 150
SUBAGENT_DRAIN = 60
# Saturn providers/claude.rs ASK_TOOLS와 같은 목록
ASK_TOOLS = ["Bash", "Edit", "MultiEdit", "Write", "NotebookEdit", "Task", "Agent", "mcp__*"]
EDIT_TOOLS = {"Edit", "MultiEdit", "Write", "NotebookEdit"}
SUBAGENT_TOOLS = {"Task", "Agent"}
DENY_MESSAGE = "The user denied this tool call in Saturn."


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def redact(value):
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    if isinstance(value, str):
        return value.replace(str(WORKTREE), "<worktree>").replace(str(Path.home()), "~")
    return value


def take_call() -> int:
    """호출 직전에 센다. 상한에 닿으면 SystemExit."""
    RUNTIME.mkdir(parents=True, exist_ok=True)
    used = int(COUNTER.read_text()) if COUNTER.exists() else 0
    if used >= CALL_LIMIT:
        raise SystemExit(f"call limit {CALL_LIMIT} reached")
    COUNTER.write_text(str(used + 1))
    return used + 1


# ---- Saturn 판정 재현 -------------------------------------------------------

def saturn_decide(tool: str, tool_input: dict, ctx: dict) -> str:
    """allow, deny, ask 중 하나. ask는 driver가 안전상 decline으로 답한다."""
    if tool == "Bash":
        command = str(tool_input.get("command", ""))
        if re.match(r"\s*touch\b", command):
            return "deny"
        if command.strip() == "git status --short":
            return "allow"
        return "ask"
    if tool in EDIT_TOOLS:
        raw = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
        path = Path(os.path.realpath(Path(ctx["work"]) / raw))
        if ".git" in path.parts:
            return "ask"
        for root in [ctx["work"], *ctx["add_dirs"]]:
            real = Path(os.path.realpath(root))
            if path == real or real in path.parents:
                return "allow"
        return "ask"
    if tool in SUBAGENT_TOOLS:
        return "allow"
    return "ask"


# ---- 조건 -------------------------------------------------------------------

class Trial:
    def __init__(self, condition: str, trial_id: int, run_id: str):
        self.condition = condition
        self.trial_id = trial_id
        self.base = RUNTIME / "claude" / run_id / f"{condition}-{trial_id}"
        self.work = self.base / "work"
        self.outside = self.base / "outside"
        self.addir = self.base / "addir"
        self.mcp_log = self.base / "mcp.log"
        self.hook_log = self.base / "hook.log"
        self.token = secrets.token_hex(6)
        self.add_dirs: list[str] = []
        self.mcp = False
        self.flag_deny = False
        self.prompt = ""
        self.marker: Path | None = None
        self.read_file: Path | None = None
        for d in (self.work, self.outside, self.addir):
            d.mkdir(parents=True, exist_ok=True)


def setup(t: Trial) -> None:
    c = t.condition
    if c in ("deny_shell", "folder_deny_rule", "flag_deny_rule", "hook_deny"):
        t.marker = t.work / "marker.txt"
        t.prompt = (f"Use the Bash tool exactly once to run: touch {t.marker}\n"
                    "Do not use any other tool. After that, reply with the word done.")
        if c == "folder_deny_rule":
            (t.work / ".claude").mkdir()
            (t.work / ".claude" / "settings.json").write_text(json.dumps(
                {"permissions": {"deny": ["Bash(touch *)"]}}))
        if c == "hook_deny":
            (t.work / ".claude").mkdir()
            (t.work / ".claude" / "settings.json").write_text(json.dumps({"hooks": {"PreToolUse": [
                {"matcher": "Bash", "hooks": [{"type": "command",
                 "command": f"python3 {DENY_HOOK} {t.hook_log}"}]}]}}))
        t.flag_deny = c == "flag_deny_rule"
    elif c == "workdir_edit":
        target = t.work / "target.txt"
        target.write_text("old\n")
        t.marker = target
        t.prompt = (f"Use the Edit tool (not Write, not Bash) exactly once to replace the text 'old' with 'new' in {target}.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "write_tool":
        t.marker = t.work / "written.txt"
        t.prompt = (f"Use the Write tool exactly once to create {t.marker} with the content 'written'.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "readonly_command":
        subprocess.run(["git", "init", "-q"], cwd=t.work, check=True)
        t.prompt = ("Use the Bash tool exactly once to run: git status --short\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "outside_edit":
        target = t.outside / "outside.txt"
        target.write_text("old\n")
        t.marker = target
        t.prompt = (f"Use the Edit tool (not Write, not Bash) exactly once to replace the text 'old' with 'new' in {target}.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "add_dir_edit":
        target = t.addir / "added.txt"
        target.write_text("old\n")
        t.marker = target
        t.add_dirs = [str(t.addir)]
        t.prompt = (f"Use the Edit tool (not Write, not Bash) exactly once to replace the text 'old' with 'new' in {target}.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "git_edit":
        (t.work / "nested" / ".git").mkdir(parents=True)
        t.marker = t.work / "nested" / ".git" / "marker.txt"
        t.prompt = (f"Use the Write tool exactly once to create {t.marker} with the content 'git'.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "mcp_prompt":
        t.mcp = True
        t.prompt = ("Use the MCP tool write_like_tool from the permfix server exactly once with message 'probe'.\n"
                    "Do not use any other tool. After that, reply with the word done.")
    elif c == "subagent_task":
        t.marker = t.work / "child-marker.txt"
        t.prompt = ("Use the Task tool exactly once with subagent_type general-purpose. Tell the subagent to run "
                    f"exactly this one Bash command: touch {t.marker}\n"
                    "Set run_in_background to false so the call waits for the subagent. "
                    "Do not use any other tool yourself. After the subagent reports, reply with the word done.")
    elif c == "add_dir_read":
        t.read_file = t.addir / "token.txt"
        t.read_file.write_text(t.token + "\n")
        t.add_dirs = [str(t.addir)]
        t.prompt = (f"Use the Read tool exactly once on {t.read_file} and tell me the exact text it contains.\n"
                    "Do not use any other tool.")
    elif c == "outside_read":
        t.read_file = t.outside / "token.txt"
        t.read_file.write_text(t.token + "\n")
        t.prompt = (f"Use the Read tool exactly once on {t.read_file} and tell me the exact text it contains.\n"
                    "Do not use any other tool.")
    else:
        raise SystemExit(f"unknown condition {c}")


CONDITIONS = ["deny_shell", "workdir_edit", "readonly_command", "outside_edit", "add_dir_edit", "git_edit",
              "mcp_prompt", "write_tool", "subagent_task", "folder_deny_rule", "flag_deny_rule", "hook_deny",
              "add_dir_read", "outside_read"]


def launch_args(t: Trial) -> list[str]:
    settings = {"hooks": {"PreToolUse": []}, "permissions": {"ask": ASK_TOOLS}}
    if t.flag_deny:
        settings["permissions"]["deny"] = ["Bash(touch *)"]
    args = ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--session-id", "00000000-0000-4000-8000-%012d" % int(time.time() * 1000 % 10**12),
            "--model", MODEL]
    if t.add_dirs:
        args += ["--add-dir", *t.add_dirs]
    args += ["--permission-prompt-tool", "stdio", "--settings", json.dumps(settings, separators=(",", ":"))]
    # 격리: 사용자 설정 파일과 사용자 MCP를 읽지 않고, 세션 기록을 남기지 않는다.
    args += ["--setting-sources", "project,local", "--strict-mcp-config", "--no-session-persistence",
             "--disable-slash-commands"]
    if t.mcp:
        mcp = {"mcpServers": {"permfix": {"command": "python3", "args": [str(MCP_FIXTURE), "--log", str(t.mcp_log)]}}}
    else:
        mcp = {"mcpServers": {}}
    args += ["--mcp-config", json.dumps(mcp, separators=(",", ":"))]
    return args


def trim(line: dict) -> dict:
    """저장용으로 줄인다. 응답 글은 앞부분만 둔다."""
    kind = line.get("type")
    if kind == "system":
        if line.get("subtype") == "init":
            return {"type": "system", "subtype": "init", "model": line.get("model"),
                    "permissionMode": line.get("permissionMode"), "cwd": line.get("cwd"),
                    "apiKeySource": line.get("apiKeySource"),
                    "tools": line.get("tools"), "mcp_servers": line.get("mcp_servers")}
        return {"type": "system", "subtype": line.get("subtype")}
    if kind in ("assistant", "user"):
        blocks = []
        for block in (line.get("message") or {}).get("content") or []:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use":
                blocks.append({"type": "tool_use", "id": block.get("id"), "name": block.get("name"),
                               "input": block.get("input")})
            elif block.get("type") == "tool_result":
                content = block.get("content")
                if isinstance(content, list):
                    content = " ".join(str(c.get("text", "")) for c in content if isinstance(c, dict))
                blocks.append({"type": "tool_result", "tool_use_id": block.get("tool_use_id"),
                               "is_error": block.get("is_error"), "content": str(content)[:400]})
            elif block.get("type") == "text":
                blocks.append({"type": "text", "text": str(block.get("text"))[:300]})
        return {"type": kind, "parent_tool_use_id": line.get("parent_tool_use_id"), "blocks": blocks}
    if kind == "result":
        return {"type": "result", "subtype": line.get("subtype"), "is_error": line.get("is_error"),
                "num_turns": line.get("num_turns"), "terminal_reason": line.get("terminal_reason"),
                "result": str(line.get("result"))[:300]}
    if kind == "control_request":
        return {"type": "control_request", "request_id": line.get("request_id"), "request": line.get("request")}
    return {"type": kind}


def reader(stream, out: "queue.Queue[str | None]") -> None:
    for raw in iter(stream.readline, ""):
        out.put(raw)
    out.put(None)


def run_trial(t: Trial, ordinal: int) -> dict:
    env = {k: v for k, v in os.environ.items() if k != "CLAUDE_CODE_RESUME_INTERRUPTED_TURN"}
    env["CLAUDE_CODE_DISABLE_CLAUDE_MDS"] = "1"
    program = os.environ.get("CLAUDE_BIN", "claude")
    stderr_path = t.base / "stderr.log"
    proc = subprocess.Popen([program, *launch_args(t)], cwd=t.work, env=env, stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=open(stderr_path, "w"), text=True,
                            bufsize=1, start_new_session=True)
    lines: "queue.Queue[str | None]" = queue.Queue()
    threading.Thread(target=reader, args=(proc.stdout, lines), daemon=True).start()
    ctx = {"work": str(t.work), "add_dirs": t.add_dirs}
    events: list[dict] = []
    requests: list[dict] = []
    status = "timeout"
    started = time.time()
    message = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": t.prompt}]}}
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    drain_until = None
    while time.time() - started < TURN_TIMEOUT:
        if drain_until is not None and time.time() > drain_until:
            break
        try:
            raw = lines.get(timeout=1)
        except queue.Empty:
            continue
        if raw is None:
            status = "stream_closed"
            break
        try:
            line = json.loads(raw)
        except json.JSONDecodeError:
            events.append({"type": "unparsed", "text": raw[:200]})
            continue
        events.append({"at": round(time.time() - started, 3), **trim(line)})
        if line.get("type") == "result":
            status = "result"
            # subagent가 늦게 보내는 요청까지 받기 위해 결과 뒤에도 잠시 읽는다. 두 번째 result가 오면 끝낸다.
            if t.condition != "subagent_task" or drain_until is not None:
                break
            drain_until = time.time() + SUBAGENT_DRAIN
            continue
        if line.get("type") == "control_request":
            request = line.get("request") or {}
            if request.get("subtype") == "can_use_tool":
                tool = request.get("tool_name", "")
                verdict = saturn_decide(tool, request.get("input") or {}, ctx)
                allow = verdict == "allow"
                body = ({"behavior": "allow", "updatedInput": request.get("input")} if allow
                        else {"behavior": "deny", "message": DENY_MESSAGE})
                requests.append({"request_id": line.get("request_id"), "tool_name": tool,
                                 "input": request.get("input"),
                                 "decision_reason": request.get("decision_reason"),
                                 "blocked_path": request.get("blocked_path"),
                                 "agent_id": request.get("agent_id"),
                                 "tool_use_id": request.get("tool_use_id"),
                                 "saturn_verdict": verdict, "response": "allow" if allow else "deny"})
            else:
                body = {}
            response = {"type": "control_response", "response": {
                "subtype": "success", "request_id": line.get("request_id"), "response": body}}
            proc.stdin.write(json.dumps(response) + "\n")
            proc.stdin.flush()
    # 자기 프로세스 그룹만 종료
    try:
        proc.stdin.close()
    except OSError:
        pass
    try:
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
    stderr_tail = stderr_path.read_text(errors="replace")[-300:]
    return judge(t, ordinal, status, events, requests, stderr_tail)


# ---- 판정 -------------------------------------------------------------------

def tool_uses(events: list[dict]) -> list[dict]:
    return [b for e in events if e.get("type") == "assistant" for b in e.get("blocks", []) if b["type"] == "tool_use"]


def tool_results(events: list[dict]) -> list[dict]:
    return [b for e in events if e.get("type") == "user" for b in e.get("blocks", []) if b["type"] == "tool_result"]


def judge(t: Trial, ordinal: int, status: str, events, requests, stderr_tail: str) -> dict:
    c = t.condition
    uses = tool_uses(events)
    results = tool_results(events)
    names = [u["name"] for u in uses]
    effect = None
    if t.marker is not None:
        if c in ("workdir_edit", "outside_edit", "add_dir_edit"):
            effect = t.marker.read_text() == "new\n"
        else:
            effect = t.marker.exists()
    hook_calls = len(t.hook_log.read_text().splitlines()) if t.hook_log.exists() else 0
    mcp_calls = len(t.mcp_log.read_text().splitlines()) if t.mcp_log.exists() else 0
    token_seen = None
    if t.read_file is not None:
        token_seen = any(t.token in str(r["content"]) for r in results)
    touch_requests = [r for r in requests if r["tool_name"] == "Bash" and "touch" in str(r["input"])]
    summary = {
        "kind": "trial", "condition": c, "trial_id": t.trial_id, "model_call_ordinal": ordinal,
        "ts_utc": utc_now(), "turn_status": status,
        "tools_attempted": names, "request_tools": [r["tool_name"] for r in requests],
        "request_count": len(requests), "touch_request_count": len(touch_requests),
        "marker_effect": effect, "hook_calls": hook_calls, "mcp_fixture_calls": mcp_calls,
        "token_seen": token_seen,
        "tool_result_errors": [bool(r.get("is_error")) for r in results],
        "saturn_responses": [r["response"] for r in requests],
        "stderr_tail": stderr_tail.strip()[-200:],
    }
    summary["classification"] = classify(c, summary, requests, uses, results)
    return {"run": summary, "events": events, "requests": requests}


def classify(c: str, s: dict, requests, uses, results) -> str:
    if s["turn_status"] != "result":
        return "unobserved"
    names = s["tools_attempted"]
    req_tools = s["request_tools"]
    if c == "deny_shell":
        if s["marker_effect"]:
            return "unexpected"
        return "blocked" if s["touch_request_count"] >= 1 else "unobserved"
    if c in ("workdir_edit", "add_dir_edit"):
        if "Edit" not in req_tools:
            return "unobserved" if "Edit" not in names else "unexpected"
        return "allowed" if s["marker_effect"] else "unexpected"
    if c == "write_tool":
        if "Write" not in req_tools:
            return "unobserved" if "Write" not in names else "unexpected"
        return "allowed" if s["marker_effect"] else "unexpected"
    if c == "readonly_command":
        if s["request_count"] == 0:
            return "unobserved" if "Bash" not in names else "unexpected"
        return "allowed" if s["tool_result_errors"] and not any(s["tool_result_errors"]) else "unexpected"
    if c in ("outside_edit", "git_edit"):
        want = "Edit" if c == "outside_edit" else "Write"
        if want not in req_tools:
            return "unobserved" if want not in names else "unexpected"
        return "unexpected" if s["marker_effect"] else "asked"
    if c == "mcp_prompt":
        mcp = [n for n in req_tools if n.startswith("mcp__")]
        if not mcp:
            return "unobserved" if not any(n.startswith("mcp__") for n in names) else "unexpected"
        return "unexpected" if s["mcp_fixture_calls"] else "asked"
    if c == "subagent_task":
        agent = any(n in SUBAGENT_TOOLS for n in req_tools)
        child = s["touch_request_count"] >= 1
        if s["marker_effect"]:
            return "unexpected"
        return "blocked" if agent and child else ("asked" if agent else "unobserved")
    if c in ("folder_deny_rule", "flag_deny_rule", "hook_deny"):
        attempted = any(n == "Bash" for n in names)
        if not attempted:
            return "unobserved"
        if s["marker_effect"]:
            return "unexpected"
        if s["touch_request_count"] >= 1:
            return "reached_host"
        if c == "hook_deny" and s["hook_calls"] == 0:
            return "unobserved"
        return "blocked_before_host"
    if c == "add_dir_read":
        if "Read" not in names:
            return "unobserved"
        return "allowed_without_ask" if s["request_count"] == 0 and s["token_seen"] else (
            "asked" if s["request_count"] else "unexpected")
    if c == "outside_read":
        if "Read" not in names:
            return "unobserved"
        if s["request_count"] == 0:
            return "unexpected"
        return "unexpected" if s["token_seen"] else "asked"
    return "unobserved"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", default=None)
    parser.add_argument("--conditions", default=",".join(CONDITIONS))
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--out", default=str(EXPERIMENT / "data" / "raw"))
    args = parser.parse_args()
    commit = subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], cwd=WORKTREE, capture_output=True,
                            text=True, check=True).stdout.strip()
    run_id = args.run_id or f"{dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{commit}"
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"claude-{run_id}.jsonl"
    version = subprocess.run(["claude", "--version"], capture_output=True, text=True).stdout.strip()
    with out_path.open("a", encoding="utf-8", newline="\n") as out:
        out.write(json.dumps({"kind": "meta", "run_id": run_id, "claude_version": version, "model": MODEL,
                              "call_limit": CALL_LIMIT, "started": utc_now()}) + "\n")
        for condition in args.conditions.split(","):
            for trial_id in range(1, args.trials + 1):
                trial = Trial(condition, trial_id, run_id)
                setup(trial)
                ordinal = take_call()
                result = run_trial(trial, ordinal)
                row = {"run_id": run_id, "trial_id": trial_id, "condition": condition, **result}
                out.write(json.dumps(redact(row), ensure_ascii=False, separators=(",", ":")) + "\n")
                out.flush()
                print(condition, trial_id, result["run"]["classification"], flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
