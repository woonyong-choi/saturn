#!/usr/bin/env python3
"""Codex 훅 입력을 기록하고 Saturn 훅 명령에 그대로 넘긴다. 사용: hook_tap.py <로그> <saturn-engine> <Saturn 홈>"""
import json
import subprocess
import sys
import time

log, binary, home = sys.argv[1:4]
data = sys.stdin.read()
proc = subprocess.run([binary, "hook", "pre-tool-use", "--home", home], input=data, capture_output=True, text=True)
try:
    parsed = json.loads(data)
except json.JSONDecodeError:
    parsed = data[:500]
entry = {"ts": time.time(), "stdin": parsed, "exit": proc.returncode, "stdout": proc.stdout[:500], "stderr": proc.stderr[:500]}
with open(log, "a", encoding="utf-8") as out:
    out.write(json.dumps(entry, ensure_ascii=False) + "\n")
sys.stdout.write(proc.stdout)
sys.stderr.write(proc.stderr)
sys.exit(proc.returncode)
