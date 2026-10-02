"""실험 3 Codex MCP elicitation fixture."""
from __future__ import annotations

import argparse
import json
import sys
import time


def send(value):
    sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("form", "url"), required=True)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    request_id = 1
    pending_call_id = None
    with open(args.log, "w", encoding="utf-8", newline="\n") as log:
        def record(direction, value):
            log.write(json.dumps({"direction": direction, "tsNs": time.time_ns(), "message": value}, ensure_ascii=False) + "\n")
            log.flush()

        for line in sys.stdin:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            record("in", message)
            method = message.get("method")
            if method == "initialize":
                value = {"jsonrpc": "2.0", "id": message.get("id"), "result": {
                    "protocolVersion": "2025-11-25", "capabilities": {"elicitation": {}},
                    "serverInfo": {"name": "experiment-252", "version": "1"}}}
                send(value)
                record("out", value)
            elif method == "tools/list":
                value = {"jsonrpc": "2.0", "id": message.get("id"), "result": {"tools": [{
                    "name": "request_experiment_input", "description": "Ask the host for experiment input.",
                    "inputSchema": {"type": "object", "properties": {"mode": {"type": "string"}}, "required": ["mode"]}}]}}
                send(value)
                record("out", value)
            elif method == "tools/call":
                pending_call_id = message.get("id")
                call_id = request_id
                request_id += 1
                if args.mode == "form":
                    params = {"mode": "form", "message": "실험 입력을 작성하라", "requestedSchema": {
                        "type": "object", "properties": {
                            "title": {"type": "string", "title": "제목"},
                            "count": {"type": "integer", "title": "횟수", "minimum": 0},
                            "enabled": {"type": "boolean", "title": "사용"},
                            "choice": {"type": "string", "title": "단일 선택", "enum": ["a", "b"]},
                            "tags": {"type": "array", "title": "다중 선택", "items": {"type": "string", "enum": ["x", "y"]}}},
                        "required": ["title", "count", "enabled", "choice", "tags"]}}
                else:
                    params = {"mode": "url", "elicitationId": "experiment-252-url", "message": "URL을 열어라",
                              "url": "https://example.invalid/experiment-252"}
                value = {"jsonrpc": "2.0", "id": call_id, "method": "elicitation/create", "params": params}
                send(value)
                record("out", value)
            elif "id" in message:
                response = message.get("result") or message.get("error")
                record("elicitation_response", response)
                result = {"content": [{"type": "text", "text": json.dumps(response, ensure_ascii=False)}], "isError": False}
                value = {"jsonrpc": "2.0", "id": pending_call_id, "result": result}
                send(value)
                record("out", value)


if __name__ == "__main__":
    main()
