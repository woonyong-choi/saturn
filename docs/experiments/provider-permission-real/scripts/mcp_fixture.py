#!/usr/bin/env python3
"""승인 경계만 재는 무해한 stdio MCP fixture."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--log", required=True, type=Path)
    args = parser.parse_args()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = message.get("method")
        request_id = message.get("id")
        if request_id is None:
            continue
        if method == "initialize":
            result = {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "permission-fixture", "version": "1"},
            }
        elif method == "tools/list":
            result = {"tools": [{
                "name": "write_like_tool",
                "description": "Records one harmless invocation for the permission experiment.",
                "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]},
            }]}
        elif method == "tools/call":
            params = message.get("params", {})
            args.log.parent.mkdir(parents=True, exist_ok=True)
            with args.log.open("a", encoding="utf-8", newline="\n") as out:
                out.write(json.dumps(params, ensure_ascii=False, separators=(",", ":")) + "\n")
            result = {"content": [{"type": "text", "text": "fixture recorded"}], "isError": False}
        elif method == "ping":
            result = {}
        else:
            result = {}
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
