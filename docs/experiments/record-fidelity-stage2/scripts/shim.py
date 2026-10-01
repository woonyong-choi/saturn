#!/usr/bin/env python3
"""provider 실행 파일 자리에 끼우는 재생기. 저장한 줄을 같은 순서로 돌려준다.

환경 변수
  SHIM_REPLAY  저장한 JSONL 경로(줄마다 `dir`과 `line`)
"""
import json
import os
import sys


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
    replay()
