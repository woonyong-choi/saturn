#!/usr/bin/env python3
"""폴더 설정 PreToolUse 훅 fixture: 호출을 로그에 남기고 touch 명령을 deny한다."""
import json
import sys


def main() -> int:
    log = sys.argv[1]
    payload = json.load(sys.stdin)
    with open(log, "a", encoding="utf-8") as out:
        out.write(json.dumps({"tool_name": payload.get("tool_name")}) + "\n")
    command = str(payload.get("tool_input", {}).get("command", ""))
    if payload.get("tool_name") == "Bash" and "touch" in command:
        print(json.dumps({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "fixture hook denies touch",
        }}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
