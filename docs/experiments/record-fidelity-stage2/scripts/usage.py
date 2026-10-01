"""주간 사용률 읽기. Claude는 `claude -p /usage`, Codex는 가장 최근 session 기록의 rate_limits를 읽는다."""
import glob
import json
import os
import re
import subprocess


def claude():
    out = subprocess.run(["claude", "-p", "/usage"], capture_output=True, text=True, timeout=60).stdout
    week = re.search(r"Current week \(all models\): (\d+(?:\.\d+)?)% used", out)
    session = re.search(r"Current session: (\d+(?:\.\d+)?)% used", out)
    return {
        "week": float(week.group(1)) if week else None,
        "session": float(session.group(1)) if session else None,
    }


def codex():
    files = glob.glob(os.path.expanduser("~/.codex/sessions/*/*/*/*.jsonl"))
    files.sort(key=os.path.getmtime, reverse=True)
    for path in files[:5]:
        used = None
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                if '"rate_limits"' not in line:
                    continue
                try:
                    limits = json.loads(line)["payload"]["rate_limits"]
                except (KeyError, ValueError, TypeError):
                    continue
                primary = (limits or {}).get("primary") or {}
                if primary.get("window_minutes") == 10080:
                    used = primary.get("used_percent")
        if used is not None:
            return {"week": used}
    return {"week": None}


def snapshot():
    return {"claude": claude(), "codex": codex()}


if __name__ == "__main__":
    print(json.dumps(snapshot()))
