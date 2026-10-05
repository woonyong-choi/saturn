"""요청 목록을 섞은 순서로 기준 judge에 보내 요청과 응답 원문을 data/raw/<단계>.jsonl에 저장한다.

사용: python3 scripts/01-collect.py dev|confirm (실험 폴더에서 실행)
필요: 환경 변수 SATURN_JUDGE_KEY (키체인 saturn-verify-router의 값, 같은 셸 명령 안에서만 읽는다)
전체 호출 시도(재시도 포함)가 plan.CALL_CAP에 닿으면 멈춘다. 라벨은 읽지 않는다.
"""
import datetime
import hashlib
import json
import os
import socket
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import plan  # noqa: E402

ROOT = plan.ROOT
RAW = ROOT / "data" / "raw"
KEY_ENV = "SATURN_JUDGE_KEY"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
SEND_RETRIES = 3
RATE_LIMIT_RETRIES = 3
RATE_LIMIT_WAIT_SECONDS = 2.0
REPLY_TIMEOUT_SECONDS = 30


def attempts_so_far() -> int:
    return sum(1 for path in RAW.glob("*.jsonl") for line in path.open(encoding="utf-8") if line.strip())


def finished(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return {json.loads(line)["trial_id"] for line in path.open(encoding="utf-8") if line.strip() and json.loads(line)["final"]}


def now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def send(data: bytes, key: str) -> tuple[int | None, str | None, str | None, str]:
    """한 번 보낸다. 상태 코드, 응답 본문, 오류 종류, 재시도 가능 여부(retry|rate|none)를 돌려준다."""
    request = urllib.request.Request(
        ENDPOINT, data=data, method="POST",
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=REPLY_TIMEOUT_SECONDS) as response:
            return response.status, response.read().decode("utf-8"), None, "none"
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        kind = "rate" if error.code in (429, 529) else "none"
        return error.code, body, f"http {error.code}", kind if kind == "none" else f"rate:{error.headers.get('retry-after') or ''}"
    except urllib.error.URLError as error:
        pre_connect = isinstance(error.reason, (ConnectionRefusedError, socket.gaierror))
        return None, None, type(error.reason).__name__, "retry" if pre_connect else "none"
    except TimeoutError:
        return None, None, "timeout", "none"


def main() -> int:
    phase = sys.argv[1] if len(sys.argv) > 1 else ""
    if phase not in ("dev", "confirm"):
        print("사용: 01-collect.py dev|confirm", file=sys.stderr)
        return 2
    key = os.environ.get(KEY_ENV)
    if not key:
        print(f"{KEY_ENV}가 없어 수집하지 않는다: Jev API 키가 필요하다", file=sys.stderr)
        return 2
    RAW.mkdir(parents=True, exist_ok=True)
    out_path = RAW / f"{phase}.jsonl"
    done = finished(out_path)
    trials = [t for t in plan.build_trials(phase) if t["trial_id"] not in done]
    total = attempts_so_far()
    redactions = 0
    with out_path.open("a", encoding="utf-8", newline="\n") as out:
        for trial in trials:
            data = json.dumps(trial["body"], ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            request_sha = hashlib.sha256(data).hexdigest()
            send_failures = rate_limits = 0
            attempt = 0
            while True:
                if total >= plan.CALL_CAP:
                    print(f"호출 상한 {plan.CALL_CAP}회에 닿아 멈춘다", file=sys.stderr)
                    return 4
                attempt += 1
                total += 1
                sent = time.monotonic()
                status, text, error, retry = send(data, key)
                latency_ms = round((time.monotonic() - sent) * 1000)
                if text and key in text:
                    text = text.replace(key, "[redacted]")
                    redactions += 1
                final = True
                if retry == "retry" and send_failures < SEND_RETRIES:
                    send_failures += 1
                    final = False
                elif retry.startswith("rate") and rate_limits < RATE_LIMIT_RETRIES:
                    rate_limits += 1
                    final = False
                reply = None
                if text:
                    try:
                        reply = json.loads(text)
                    except json.JSONDecodeError:
                        reply = None
                row = {
                    "trial_id": trial["trial_id"], "phase": phase, "group": trial["group"], "role": trial["role"],
                    "item_id": trial["item_id"], "variant": trial["variant"], "rep": trial["rep"],
                    "option_order": trial["option_order"], "attempt": attempt, "final": final,
                    "request": trial["body"], "request_sha256": request_sha, "ts_utc": now(),
                    "http_status": status, "latency_ms": latency_ms, "error": error,
                    "response": reply, "response_text": None if reply is not None else text,
                }
                out.write(json.dumps(row, ensure_ascii=False) + "\n")
                out.flush()
                if status in (401, 403):
                    print(f"키가 거절되어 멈춘다: http {status}", file=sys.stderr)
                    return 3
                if final:
                    break
                if retry.startswith("rate"):
                    wait = retry.split(":", 1)[1]
                    time.sleep(float(wait) if wait else RATE_LIMIT_WAIT_SECONDS)
    print(f"수집 끝: {phase} 시도 누계 {total}회, 키 일치 치환 {redactions}회")
    return 0


if __name__ == "__main__":
    sys.exit(main())
