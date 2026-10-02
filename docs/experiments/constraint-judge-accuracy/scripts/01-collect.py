"""평가 세트를 기준 judge에 물어 답을 data/raw/에 그대로 저장한다.

사용: python3 scripts/01-collect.py (실험 폴더에서 실행)
필요: 환경 변수 SATURN_JUDGE_KEY (Jev API 키)
"""
import datetime
import hashlib
import json
import os
import platform
import random
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVAL = ROOT / "eval"
RAW = ROOT / "data" / "raw"
KEY_ENV = "SATURN_JUDGE_KEY"
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-1.13.0"
ORDER_SEED = 121
SOURCE = "jev"
# router.md 재시도 기본값과 같다.
SEND_RETRIES = 3
RATE_LIMIT_RETRIES = 3
RATE_LIMIT_WAIT_SECONDS = 2.0
REPLY_TIMEOUT_SECONDS = 30


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build_trials(questions: dict) -> list[dict]:
    """두 질문의 시행을 하나로 섞어 시드 순서로 묻는다."""
    trials = []
    for item in read_jsonl(EVAL / "inputs.jsonl"):
        trials.append({
            "condition": "is_constraint",
            "item_id": item["id"],
            "question_id": "is_constraint",
            "state": {"previous_user_input": item["previous"], "latest_user_input": item["text"]},
        })
    for item in read_jsonl(EVAL / "pairs.jsonl"):
        trials.append({
            "condition": "replaces",
            "item_id": item["id"],
            "question_id": "replaces_1",
            "state": {"earlier_constraint": item["earlier"], "later_constraint": item["later"]},
        })
    random.Random(ORDER_SEED).shuffle(trials)
    for trial in trials:
        question = questions[trial["question_id"]]
        trial["question_set"] = question["set"]
        trial["body"] = {
            "model": MODEL,
            "state": trial["state"],
            "questions": {trial["question_id"]: {"type": question["type"], "instructions": question["instructions"]}},
        }
    return trials


def post(body: dict, key: str) -> tuple[int | None, dict | None, str | None]:
    """보내기 전 실패와 속도 제한만 다시 보낸다. 응답 본문의 키 문자열은 남기지 않는다."""
    data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    send_failures = 0
    rate_limits = 0
    while True:
        request = urllib.request.Request(
            ENDPOINT,
            data=data,
            method="POST",
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=REPLY_TIMEOUT_SECONDS) as response:
                return response.status, json.loads(response.read().decode("utf-8")), None
        except urllib.error.HTTPError as error:
            if error.code in (429, 529) and rate_limits < RATE_LIMIT_RETRIES:
                rate_limits += 1
                time.sleep(float(error.headers.get("retry-after") or RATE_LIMIT_WAIT_SECONDS))
                continue
            return error.code, None, f"http {error.code}"
        except urllib.error.URLError as error:
            # 연결 전 실패만 보내기 전 실패로 본다.
            if isinstance(error.reason, (ConnectionRefusedError, socket.gaierror)) and send_failures < SEND_RETRIES:
                send_failures += 1
                continue
            return None, None, type(error.reason).__name__
        except TimeoutError:
            return None, None, "timeout"


def read_answer(reply: dict | None, question_id: str) -> tuple[str, float | None]:
    if reply is None:
        return "no_answer", None
    answer = (reply.get("answers") or {}).get(question_id) or {}
    value = answer.get("noul")
    if not isinstance(value, (int, float)) or not 0.0 <= value <= 1.0:
        return "invalid", None
    return "ok", float(value)


def write_env(run_id: str, commit: str, started: str, models: list[str]) -> None:
    env = json.loads((ROOT / "env.json").read_text(encoding="utf-8"))
    memory = None
    if hasattr(os, "sysconf") and "SC_PHYS_PAGES" in os.sysconf_names:
        memory = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    env.update({
        "run_id": run_id,
        "commit": commit,
        "date_utc": started,
        "os": f"{platform.system()} {platform.release()}",
        "cpu": platform.processor() or platform.machine(),
        "memory_bytes": memory,
        "python": platform.python_version(),
        "judge_models_answered": models,
        "eval_sha256": {name: sha256(EVAL / name) for name in ("inputs.jsonl", "pairs.jsonl", "questions.json")},
    })
    (ROOT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_checksums() -> None:
    files = sorted(RAW.glob("*.jsonl")) + [EVAL / "inputs.jsonl", EVAL / "pairs.jsonl", EVAL / "questions.json"]
    lines = [f"{sha256(path)}  {path.relative_to(ROOT).as_posix()}" for path in files]
    (ROOT / "data" / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    key = os.environ.get(KEY_ENV)
    if not key:
        print(f"{KEY_ENV}가 없어 수집하지 않는다: Jev API 키가 필요하다", file=sys.stderr)
        return 2
    commit = git("rev-parse", "--short=7", "HEAD")
    now = datetime.datetime.now(datetime.timezone.utc)
    started = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    run_id = f"{now.strftime('%Y%m%dT%H%M%SZ')}-{commit}"
    RAW.mkdir(parents=True, exist_ok=True)
    out_path = RAW / f"{SOURCE}-{run_id}.jsonl"
    questions = json.loads((EVAL / "questions.json").read_text(encoding="utf-8"))
    trials = build_trials(questions)
    models = set()
    with out_path.open("x", encoding="utf-8", newline="\n") as out:
        for index, trial in enumerate(trials, start=1):
            sent = time.monotonic()
            status_code, reply, error = post(trial["body"], key)
            if status_code in (401, 403):
                out.close()
                out_path.unlink()
                print(f"키가 거절되어 수집을 멈춘다: http {status_code}", file=sys.stderr)
                return 3
            latency_ms = round((time.monotonic() - sent) * 1000)
            status, answer = read_answer(reply, trial["question_id"])
            model = reply.get("model") if reply else None
            if model:
                models.add(model)
            row = {
                "run_id": run_id,
                "trial_id": f"t-{index:04d}",
                "condition": trial["condition"],
                "ts_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "item_id": trial["item_id"],
                "question_set": trial["question_set"],
                "question_id": trial["question_id"],
                "status": status,
                "http_status": status_code,
                "answer": answer,
                "model": model,
                "latency_ms": latency_ms,
                "error": error,
            }
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
    write_env(run_id, commit, started, sorted(models))
    write_checksums()
    print(f"수집 끝: {out_path.relative_to(ROOT)} {len(trials)}건")
    return 0


if __name__ == "__main__":
    sys.exit(main())
