"""Collect per-turn numbers from local Claude Code and Codex logs.

Writes only anonymous ids and numbers. No text, path, project name, or file
name leaves this process. Session ids are hashed with a random salt that is
never stored, so ids cannot be linked back to the source logs.
"""

import argparse
import glob
import hashlib
import json
import os
import platform
import secrets
import subprocess
from datetime import datetime, timezone

CLAUDE_GLOB = os.path.expanduser("~/.claude/projects/**/*.jsonl")
CODEX_GLOB = os.path.expanduser("~/.codex/sessions/**/*.jsonl")
CODEX_PREAMBLE = ("<environment_context>", "<user_instructions>", "# AGENTS.md", "<INSTRUCTIONS>")
CLAUDE_NON_PROMPT = ("<command-", "<local-command", "[Request interrupted", "<bash-", "Caveat:")


def text_len(value):
    if value is None:
        return 0
    if isinstance(value, str):
        return len(value)
    return len(json.dumps(value, ensure_ascii=False))


def new_turn():
    return {"tool_calls": 0, "tool_io_chars": 0, "context_tokens": None, "compacted": False}


def claude_prompt_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        if any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content):
            return None
        parts = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
        return "".join(parts) if parts else None
    return None


def parse_claude(path, stats):
    turns = []
    first_key = None
    current = None
    pending_compact = False
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                stats["bad_lines"] += 1
                continue
            if not isinstance(e, dict) or e.get("isSidechain"):
                continue
            kind = e.get("type")
            if kind == "system" and e.get("subtype") == "compact_boundary":
                if current is None:
                    pending_compact = True
                else:
                    current["compacted"] = True
                continue
            msg = e.get("message") or {}
            if kind == "user":
                if e.get("isCompactSummary"):
                    pending_compact = True
                    if current is not None:
                        current["compacted"] = True
                    continue
                content = msg.get("content")
                if isinstance(content, list):
                    for b in content:
                        if isinstance(b, dict) and b.get("type") == "tool_result" and current is not None:
                            current["tool_io_chars"] += text_len(b.get("content"))
                text = claude_prompt_text(content)
                if text is None or e.get("isMeta") or text.lstrip().startswith(CLAUDE_NON_PROMPT):
                    continue
                current = new_turn()
                if pending_compact:
                    current["compacted"] = True
                    pending_compact = False
                turns.append(current)
                if first_key is None:
                    first_key = e.get("uuid") or e.get("sessionId")
            elif kind == "assistant" and current is not None:
                for b in msg.get("content") or []:
                    if isinstance(b, dict) and b.get("type") == "tool_use":
                        current["tool_calls"] += 1
                        current["tool_io_chars"] += text_len(b.get("input"))
                usage = msg.get("usage")
                if isinstance(usage, dict) and "input_tokens" in usage:
                    current["context_tokens"] = sum(
                        int(usage.get(k) or 0)
                        for k in ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens")
                    )
    return first_key, turns


def codex_message_text(payload):
    parts = []
    for b in payload.get("content") or []:
        if isinstance(b, dict):
            parts.append(b.get("text") or "")
    return "".join(parts)


def parse_codex(path, stats):
    turns = []
    first_key = None
    current = None
    pending_compact = False
    has_user_event = False
    items = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                stats["bad_lines"] += 1
                continue
            if not isinstance(e, dict):
                continue
            if "payload" in e and isinstance(e.get("payload"), dict):
                items.append((e.get("type"), e["payload"]))
            else:
                items.append(("response_item", e))
            if items[-1][0] == "event_msg" and items[-1][1].get("type") == "user_message":
                has_user_event = True
    for kind, p in items:
        ptype = p.get("type")
        if kind == "session_meta":
            source = p.get("source")
            if isinstance(source, dict) and "subagent" in json.dumps(source):
                return None, "subagent"
            first_key = first_key or p.get("id")
            continue
        if kind == "compacted" or (kind == "event_msg" and ptype == "context_compacted"):
            if current is None:
                pending_compact = True
            else:
                current["compacted"] = True
            continue
        is_prompt = False
        if has_user_event:
            is_prompt = kind == "event_msg" and ptype == "user_message"
        elif kind == "response_item" and ptype == "message" and p.get("role") == "user":
            is_prompt = not codex_message_text(p).lstrip().startswith(CODEX_PREAMBLE)
        if is_prompt:
            current = new_turn()
            if pending_compact:
                current["compacted"] = True
                pending_compact = False
            turns.append(current)
            continue
        if current is None:
            continue
        if kind == "response_item":
            if ptype in ("function_call", "custom_tool_call", "local_shell_call", "web_search_call"):
                current["tool_calls"] += 1
                current["tool_io_chars"] += text_len(p.get("arguments") or p.get("input") or p.get("action"))
            elif ptype in ("function_call_output", "custom_tool_call_output"):
                current["tool_io_chars"] += text_len(p.get("output"))
        elif kind == "event_msg" and ptype == "token_count":
            last = ((p.get("info") or {}).get("last_token_usage")) or {}
            if "input_tokens" in last:
                current["context_tokens"] = int(last.get("input_tokens") or 0) + int(last.get("output_tokens") or 0)
    return first_key, turns


def collect(provider, pattern, parser, stats, salt, run_id, ts):
    best = {}
    for path in sorted(glob.glob(pattern, recursive=True)):
        if provider == "claude" and os.sep + "subagents" + os.sep in path:
            stats["excluded_subagent_files"] += 1
            continue
        stats["files"] += 1
        try:
            key, turns = parser(path, stats)
        except (OSError, ValueError, AttributeError, TypeError):
            stats["failed_files"] += 1
            continue
        if turns == "subagent":
            stats["excluded_subagent_files"] += 1
            continue
        if not turns:
            stats["excluded_no_turns"] += 1
            continue
        if all(t["context_tokens"] is None for t in turns):
            stats["excluded_no_tokens"] += 1
            continue
        key = key or path
        anon = hashlib.sha256((salt + provider + str(key)).encode()).hexdigest()[:16]
        if anon in best:
            stats["excluded_duplicate"] += 1
            if len(best[anon]["turns"]) >= len(turns):
                continue
        best[anon] = {
            "run_id": run_id,
            "trial_id": anon,
            "condition": "observed",
            "ts_utc": ts,
            "session_id": anon,
            "provider": provider,
            "turns": turns,
        }
    return list(best.values())


def command_output(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=20, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.strip() or None


def write_env(path, run_id, ts):
    try:
        import numpy
        numpy_version = numpy.__version__
    except ImportError:
        numpy_version = None
    mem = command_output(["sysctl", "-n", "hw.memsize"])
    env = {
        "os": platform.platform(),
        "cpu": command_output(["sysctl", "-n", "machdep.cpu.brand_string"]) or platform.processor(),
        "memory_bytes": int(mem) if mem and mem.isdigit() else None,
        "python": platform.python_version(),
        "numpy": numpy_version,
        "claude_code": command_output(["claude", "--version"]),
        "codex": command_output(["codex", "--version"]),
        "model": "해당 없음",
        "run_id": run_id,
        "date_utc": ts,
        "commit": run_id.split("-")[-1],
        "seed": 119,
    }
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(env, f, ensure_ascii=False, indent=2)
        f.write("\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-id", required=True)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--env", required=True)
    args = ap.parse_args()
    salt = secrets.token_hex(32)
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    meta = {"run_id": args.run_id, "ts_utc": ts}
    rows = []
    for provider, pattern, parser in (("claude", CLAUDE_GLOB, parse_claude), ("codex", CODEX_GLOB, parse_codex)):
        stats = dict.fromkeys(
            ("files", "failed_files", "bad_lines", "excluded_subagent_files", "excluded_no_turns",
             "excluded_no_tokens", "excluded_duplicate"), 0)
        found = collect(provider, pattern, parser, stats, salt, args.run_id, ts)
        stats["sessions"] = len(found)
        meta[provider] = stats
        rows.extend(found)
    del salt
    write_env(args.env, args.run_id, ts)
    rows.sort(key=lambda r: r["session_id"])
    os.makedirs(args.out_dir, exist_ok=True)
    out = os.path.join(args.out_dir, f"agent-logs-{args.run_id}.jsonl")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False, separators=(",", ":")) + "\n")
    with open(os.path.join(args.out_dir, f"agent-logs-{args.run_id}.meta.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(meta, f, indent=2, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
