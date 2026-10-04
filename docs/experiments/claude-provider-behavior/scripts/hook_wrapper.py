#!/usr/bin/env python3
"""PreToolUse 훅 감싸기: 입력 요약을 로그에 남기고, 같은 입력을 Saturn engine의 `hook pre-tool-use`에 넘겨 결과를 그대로 돌려준다."""
import json
import subprocess
import sys


def main() -> int:
    log, saturn_bin, saturn_home = sys.argv[1:4]
    raw = sys.stdin.read()
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        payload = {}
    tool_input = payload.get("tool_input") or {}
    row = {"keys": sorted(payload.keys()), "tool_name": payload.get("tool_name"),
           "agent_id": payload.get("agent_id"), "agent_type": payload.get("agent_type"),
           "hook_event_name": payload.get("hook_event_name"),
           "command": str(tool_input.get("command", ""))[:200] or None}
    done = subprocess.run([saturn_bin, "hook", "pre-tool-use", "--home", saturn_home], input=raw,
                          capture_output=True, text=True)
    row["exit_code"] = done.returncode
    row["denied"] = '"permissionDecision":"deny"' in done.stdout.replace(" ", "")
    with open(log, "a", encoding="utf-8") as out:
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
    sys.stdout.write(done.stdout)
    sys.stderr.write(done.stderr)
    return done.returncode


if __name__ == "__main__":
    raise SystemExit(main())
