"""Jev 패킷 선별 판단(`compact`)의 반복 일관성. #544가 다루지 않은 역할이다.

사용:
  python3 04-replay.py collect   기록된 `compact` 요청을 그대로 5번 다시 보낸다(키는 키체인에서 이 명령 안에서만 읽는다)
  python3 04-replay.py analyze [--check]

요청은 trial의 기록 저장소에 남은 `judgments.sent` 원문이다. 응답만 새로 받고 요청은 바꾸지 않는다.
"""

from __future__ import annotations

import gzip
import json
import math
import socket
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import engine_driver

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
RAW = EXP / "data" / "raw"
REPLAY = EXP / "data" / "replay"
OUT = EXP / "results" / "compact-consistency.json"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
REPEATS = 5
SPREAD_LIMIT = 0.10  # jev-decision-consistency H3, H4와 같은 확률 폭 기준
WILSON_LOWER_REQUIRED = 0.90  # jev-decision-consistency H1, H2와 같은 일관성 기준
TIMEOUT_SECONDS = 30
RATE_WAIT_SECONDS = 2.0
RATE_RETRIES = 3
CALL_CAP = 150
Z = 1.959963984540054


def requests_from_raw() -> list[dict]:
    out = []
    for path in sorted(RAW.glob("formal-*-jev.json.gz")):
        with gzip.open(path, "rt", encoding="utf-8") as f:
            res = json.load(f)
        store = res["store"]
        packets = [p for p in store["handoff_packets"] if p["kind"] in ("Restart", "Switch")]
        if not packets:
            continue
        packet = packets[-1]
        k = sum(1 for i in store["handoff_packet_items"] if i["packet_id"] == packet["id"] and i["selector"] == "compact" and i["form"])
        for j in store["judgments"]:
            if j["input_id"] is None and j["outcome"] == "Ok" and j["sent"]:
                out.append({"trial": res["trial"], "judgment_id": j["id"], "sent": j["sent"], "recorded": j["received"], "k": k})
    return out


def send(data: bytes, key: str) -> tuple[int | None, str | None, str | None]:
    request = urllib.request.Request(ENDPOINT, data=data, method="POST", headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            return response.status, response.read().decode("utf-8"), None
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", errors="replace"), f"http {error.code}"
    except (urllib.error.URLError, socket.timeout) as error:
        return None, None, type(getattr(error, "reason", error)).__name__


def collect() -> int:
    reqs = requests_from_raw()
    REPLAY.mkdir(parents=True, exist_ok=True)
    key = engine_driver.router_key()
    attempts = sum(1 for p in REPLAY.glob("*.jsonl") for _ in p.open())
    for req in reqs:
        path = REPLAY / f"{req['trial']}.jsonl"
        have = sum(1 for _ in path.open()) if path.exists() else 0
        with path.open("a", encoding="utf-8") as out:
            for n in range(have, REPEATS):
                for retry in range(RATE_RETRIES + 1):
                    if attempts >= CALL_CAP:
                        print("call cap reached", flush=True)
                        return 1
                    attempts += 1
                    status, body, error = send(req["sent"].encode("utf-8"), key)
                    if status in (429, 529) and retry < RATE_RETRIES:
                        time.sleep(RATE_WAIT_SECONDS)
                        continue
                    break
                out.write(json.dumps({"trial": req["trial"], "repeat": n, "status": status, "error": error, "received": body}, ensure_ascii=False, sort_keys=True) + "\n")
    print(f"requests {len(reqs)}, attempts {attempts}")
    return 0


def probs(received: str | None) -> dict[str, float] | None:
    if not received:
        return None
    try:
        answers = json.loads(received)["answers"]
        return {name: float(a["noul"]) for name, a in answers.items()}
    except (KeyError, ValueError, TypeError):
        return None


def item_scores(p: dict[str, float]) -> dict[int, float]:
    """항목마다 호출과 결과 중 큰 남김 확률. 질문 이름은 call_<번호>_keep, result_<번호>_keep."""
    out: dict[int, float] = {}
    for name, value in p.items():
        seq = int(name.split("_")[1])
        out[seq] = max(out.get(seq, 0.0), value)
    return out


def topk(scores: dict[int, float], k: int) -> tuple[int, ...]:
    ranked = sorted(scores, key=lambda s: (-scores[s], s))
    return tuple(sorted(ranked[:k]))


def wilson(k: int, n: int) -> tuple[float, float]:
    if n == 0:
        return 0.0, 1.0
    p = k / n
    d = 1 + Z * Z / n
    c = (p + Z * Z / (2 * n)) / d
    h = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def analyze(check: bool) -> int:
    rows = []
    for req in requests_from_raw():
        path = REPLAY / f"{req['trial']}.jsonl"
        runs = [probs(req["recorded"])]
        failed = 0
        if path.exists():
            for line in path.open(encoding="utf-8"):
                rec = json.loads(line)
                p = probs(rec["received"]) if rec["status"] == 200 else None
                if p is None:
                    failed += 1
                runs.append(p)
        good = [p for p in runs if p is not None]
        complete = len(good) == REPEATS + 1 and all(set(p) == set(good[0]) for p in good)
        row = {"trial": req["trial"], "k": req["k"], "responses": len(good), "failed": failed, "complete": complete}
        if complete:
            names = sorted(good[0])
            spread = max(max(p[n] for p in good) - min(p[n] for p in good) for n in names)
            sets = {topk(item_scores(p), req["k"]) for p in good}
            row.update(max_spread=round(spread, 4), spread_ok=spread <= SPREAD_LIMIT, same_selection=len(sets) == 1)
        rows.append(row)
    done = [r for r in rows if r["complete"]]
    sel = sum(1 for r in done if r["same_selection"])
    spr = sum(1 for r in done if r["spread_ok"])
    n = len(rows)  # 못 받은 요청은 불일치로 센다
    lo_sel = wilson(sel, n)
    lo_spr = wilson(spr, n)
    result = {
        "requests": n, "complete": len(done),
        "same_selection": {"k": sel, "n": n, "wilson95": [round(lo_sel[0], 4), round(lo_sel[1], 4)]},
        "spread_within_limit": {"k": spr, "n": n, "wilson95": [round(lo_spr[0], 4), round(lo_spr[1], 4)]},
        "required_wilson_lower": WILSON_LOWER_REQUIRED,
        "meets_design_threshold": bool(n and lo_sel[0] >= WILSON_LOWER_REQUIRED and lo_spr[0] >= WILSON_LOWER_REQUIRED),
        "per_request": rows,
    }
    text = json.dumps(result, ensure_ascii=False, sort_keys=True, indent=1) + "\n"
    if check:
        same = OUT.exists() and OUT.read_text() == text
        print("replay analysis byte-identical" if same else "replay analysis MISMATCH")
        return 0 if same else 1
    OUT.write_text(text)
    print(json.dumps({k: v for k, v in result.items() if k != "per_request"}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "collect":
        sys.exit(collect())
    if cmd == "analyze":
        sys.exit(analyze("--check" in sys.argv))
    print("usage: 04-replay.py collect|analyze [--check]", file=sys.stderr)
    sys.exit(2)
