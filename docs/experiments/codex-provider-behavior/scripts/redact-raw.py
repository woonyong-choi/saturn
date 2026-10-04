#!/usr/bin/env python3
"""수집한 raw에서 개인 정보만 지운다. 수집 직후 한 번 실행하고 그 뒤 raw는 바꾸지 않는다.

- `mcpServerStatus/list` 응답: 서버별 `tools`, `resources`, `resourceTemplates`를 개수로 바꾼다(계정 연동 서비스의 도구 이름 목록이라 개인 정보다).
- OS 사용자 이름을 `user`로 바꾸고 홈 경로를 `~`로 바꾼다(명령 출력의 `ls -l` 줄과 JSON 키 안의 경로).
"""
import glob
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USER = os.environ.get("USER", "")


def clean_event(event):
    message = event.get("msg", {})
    data = ((message.get("result") or {}).get("data")) if isinstance(message.get("result"), dict) else None
    if isinstance(data, list) and data and isinstance(data[0], dict) and "tools" in data[0]:
        for server in data:
            for key in ("tools", "resources", "resourceTemplates"):
                if isinstance(server.get(key), (dict, list)):
                    server[key] = {"_redacted_count": len(server[key])}


for path in sorted(glob.glob(str(ROOT / "data" / "raw" / "*.jsonl"))):
    if path.endswith("-calls.jsonl"):
        continue
    lines = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        for event in row.get("events", []):
            clean_event(event)
        text = json.dumps(row, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        if USER:
            text = text.replace(f"{USER}  staff", "user  staff").replace(f"/Users/{USER}", "~")
        lines.append(text)
    Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8")
print("redacted")
