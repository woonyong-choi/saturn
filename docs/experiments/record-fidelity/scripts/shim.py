#!/usr/bin/env python3
"""provider 실행 파일 자리에 끼우는 중계기. 기록 모드는 실제 provider를 실행하며 오가는 줄을 저장하고, 재생 모드는 저장한 줄을 같은 순서로 돌려준다.

환경 변수
  SHIM_REAL    기록 모드: 실제 provider 실행 파일
  SHIM_RAW     기록 모드: 줄을 저장할 JSONL 경로
  SHIM_EXTRA   기록 모드: provider 인자 끝에 붙일 인자(JSON 배열)
  SHIM_REDACT  기록 모드: 저장 전에 바꿀 [찾는 글, 바꿀 글] 쌍(JSON 배열). 저장 전 `clean`이 개인 설정과 계정 줄도 지운다
  SHIM_REPLAY  재생 모드: 저장한 JSONL 경로
"""
import json
import os
import subprocess
import sys
import threading


DROP_METHODS = ("mcpServer/", "hook/", "account/", "remoteControl/", "warning")
BLANK_INIT_KEYS = ("tools", "mcp_servers", "slash_commands", "skills", "plugins", "agents", "capabilities")
DROP_INIT_KEYS = ("memory_paths", "messaging_socket_path")


def redact(text, pairs):
    for old, new in pairs:
        text = text.replace(old, new)
    return text


def clean(direction, text):
    """저장 전에 공개할 수 없는 값을 지운다. 버릴 줄이면 None.

    Saturn 변환이 읽는 값(도구 항목, 사용량, 턴 시작과 끝, 모델, 권한 방식)은 바꾸지 않는다.
    """
    if direction != "out":
        return text
    try:
        message = json.loads(text)
    except ValueError:
        return text
    method = message.get("method", "")
    if method.startswith(DROP_METHODS) or message.get("type") == "rate_limit_event":
        return None
    result = message.get("result")
    if isinstance(result, dict):
        if "userAgent" in result:
            result["userAgent"] = "codex-app-server"
        if "instructionSources" in result:
            result["instructionSources"] = []
        for entry in result.get("data", []) if isinstance(result.get("data"), list) else []:
            if isinstance(entry, dict) and "skills" in entry:
                entry["skills"] = []
    if message.get("type") == "system" and message.get("subtype") == "init":
        for key in BLANK_INIT_KEYS:
            if key in message:
                message[key] = []
        for key in DROP_INIT_KEYS:
            message.pop(key, None)
    return json.dumps(message, ensure_ascii=False, separators=(",", ":"))


def record():
    pairs = json.loads(os.environ.get("SHIM_REDACT", "[]"))
    extra = json.loads(os.environ.get("SHIM_EXTRA", "[]"))
    lock = threading.Lock()
    raw = open(os.environ["SHIM_RAW"], "w", encoding="utf-8")

    def log(direction, line):
        kept = clean(direction, line.rstrip("\n"))
        if kept is None:
            return
        with lock:
            raw.write(json.dumps({"dir": direction, "line": redact(kept, pairs)}, ensure_ascii=False) + "\n")
            raw.flush()

    child = subprocess.Popen(
        [os.environ["SHIM_REAL"], *sys.argv[1:], *extra],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )

    def pump_in():
        for line in sys.stdin:
            log("in", line)
            try:
                child.stdin.write(line)
                child.stdin.flush()
            except OSError:
                break
        try:
            child.stdin.close()
        except OSError:
            pass

    threading.Thread(target=pump_in, daemon=True).start()
    for line in child.stdout:
        log("out", line)
        sys.stdout.write(line)
        sys.stdout.flush()
    child.wait()
    raw.close()


def replay():
    with open(os.environ["SHIM_REPLAY"], encoding="utf-8") as handle:
        rows = [json.loads(line) for line in handle if line.strip()]
    for row in rows:
        if row["dir"] == "in":
            if sys.stdin.readline() == "":
                break
        else:
            sys.stdout.write(row["line"] + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    if "SHIM_REPLAY" in os.environ:
        replay()
    else:
        record()
