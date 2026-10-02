"""합성 시나리오로 세 조건의 맥락을 만들고 새 provider session의 답을 수집한다.

사용: python3 scripts/01-collect.py [--limit N] [--resume 실행 id] (실험 폴더에서 실행)
  --limit N        시드로 섞은 시나리오 순서에서 앞의 N개만 수집한다(사용량 시험용).
  --resume 실행 id  같은 실행 id의 raw 파일에 이어 쓰고, 끝난 세션은 건너뛴다.
조건이 갖춰지지 않으면 원인 한 줄을 쓰고 종료 코드 2로, 사용량 한도에 닿으면 3으로 끝난다.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import hashlib
import json
import math
import os
import platform
import random
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

SEED = 249
SCENARIO_COUNT = 24
RRF_K = 60
PACKET_BUDGET_TOKENS = 8000
SESSION_TIMEOUT_S = 300
MAX_CONSECUTIVE_FAILURES = 5
CONDITIONS = ["last-input", "rule", "summary"]
PROVIDERS = ["claude", "codex"]
PROVIDER_MODELS = {"claude": "claude-haiku-4-5-20251001", "codex": "gpt-5.6-luna"}
SUMMARY_MODEL = "gpt-5.6-luna"
RESULT_LIMIT_CHARS = 4000
BASE_TURNS = 3
MAX_PROVIDER_CALLS = 150
MAX_SUMMARY_CALLS = 30
EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"
WORKDIR = EXPERIMENT / "data" / "provider-workdir"
DEFAULT_PACKET_CMD = "cargo run -q -p saturn-core --example packet --"

MODULES = [
    {"ko": "결제", "en": "payment", "syn": "청구"},
    {"ko": "로그인", "en": "login", "syn": "인증"},
    {"ko": "장바구니", "en": "cart", "syn": "담기 목록"},
    {"ko": "검색", "en": "search", "syn": "조회"},
    {"ko": "알림", "en": "notification", "syn": "통지"},
    {"ko": "배송", "en": "shipping", "syn": "발송"},
]
RELATIONS = ["same", "translation", "synonym"]
SESSION_DATES = ["2026-03-02", "2026-03-09", "2026-03-16", "2026-03-23"]
ANSWER_RULES = (
    "아래 맥락만 근거로 질문에 답하라. 파일을 읽거나 명령을 실행하지 마라. "
    "맥락에 근거가 없으면 unknown을 true로 둔다. 숫자는 숫자만, 날짜는 YYYY-MM-DD로 쓴다. "
    '답은 JSON 배열 하나로만 쓴다: [{"id": "q1", "answer": "...", "unknown": false}]'
)


def fail(reason: str, code: int = 2) -> None:
    print(f"01-collect: {reason}", file=sys.stderr)
    sys.exit(code)


def now_utc() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def redact(text: str) -> str:
    key = os.environ.get("SATURN_JUDGE_KEY", "")
    return text.replace(key, "[secret]") if key else text


# ---------------------------------------------------------------- 시나리오

def filler(rng: random.Random, module: dict, size: int) -> str:
    lines = []
    while sum(len(line) + 1 for line in lines) < size:
        lines.append(
            f"[{rng.randint(0, 23):02d}:{rng.randint(0, 59):02d}] {module['en']} worker "
            f"step={rng.randint(1, 999)} ok latency_ms={rng.randint(3, 900)}"
        )
    return "\n".join(lines)


def term(module: dict, relation: str) -> str:
    return {"same": module["ko"], "translation": module["en"], "synonym": module["syn"]}[relation]


def relation_of(module: dict, text: str) -> str:
    return next(r for r in RELATIONS if text.startswith(term(module, r)))


def build_facts(rng: random.Random, module: dict) -> tuple[list[dict], list[dict], dict]:
    relations = [RELATIONS[i % 3] for i in range(8)]
    rng.shuffle(relations)
    timeout = rng.randint(1200, 9800)
    retries = rng.randint(2, 9)
    code_a, code_b = f"E{rng.randint(100, 499)}", f"E{rng.randint(500, 999)}"
    old_limit, new_limit = rng.randint(10, 40), rng.randint(41, 90)
    pending = rng.choice(["결제 회귀 테스트", "로그인 오류 재현", "검색 색인 검증"])
    paused = rng.choice(["알림 배포", "배송 지표 정리", "장바구니 성능 측정"])
    unknown = rng.choice(["캐시 무효화 결과", "권한 변경 결과", "배치 재실행 결과"])
    facts = [
        (0, 1, "read", f"{term(module, relations[0])} timeout_ms = {timeout}"),
        (1, 1, "read", f"{term(module, relations[1])} retry_count = {retries}"),
        (2, 0, "shell", f"{term(module, relations[2])} error code {code_a}"),
        (3, 2, "shell", f"{term(module, relations[3])} error code {code_b}"),
        (4, 0, "edit", f"{term(module, relations[4])} max_items = {old_limit}"),
        (5, 2, "edit", f"{term(module, relations[5])} max_items = {new_limit}"),
        (6, 1, "test", f"{term(module, relations[6])} tests: {rng.randint(2, 19)} failed"),
        (7, 2, "read", f"{term(module, relations[7])} retry_count = {retries + 1}"),
    ]
    fact_items = [
        {"fact": i, "session": s, "tool": tool, "text": text, "relation": relation_of(module, text)}
        for i, (_, s, tool, text) in enumerate(facts)
    ]
    ko = module["ko"]
    questions = [
        ("goal", f"처음 시작한 {ko} 작업의 목표는 무엇인가", [f"{ko} 결제 흐름의 timeout_ms를 조정한다"], [], []),
        ("goal", f"처음 목표와 마지막 {ko} 지시는 무엇을 다루나", [ko, "마무리"], [], []),
        ("recent", "가장 최근에 받은 지시는 무엇인가", ["마무리", "실패"], [], []),
        ("recent", f"최근 지시에서 {ko}에 대해 하라는 일은 무엇인가", ["마무리", "실패"], [], []),
        ("open", "대기 중인 작업은 무엇인가", [pending], [], []),
        ("open", "보류 중인 작업은 무엇인가", [paused], [], []),
        ("unknown-result", "결과를 확인하지 못한 작업은 어떻게 다뤄야 하나", [unknown, "확인"], [], []),
        ("unknown-result", "결과를 모르는 작업을 완료된 것으로 말해도 되나", ["모른다"], [], []),
        ("contrast", f"{ko} timeout_ms는 몇 ms이고 재시도 횟수는 몇 번인가", [str(timeout), str(retries)], [], [0, 1]),
        ("contrast", f"{ko} max_items의 현재 값과 오류 코드 두 개는", [str(new_limit), code_a, code_b], [str(old_limit)], [2, 3, 4, 5]),
    ]
    return fact_items, [
        {"qid": f"q{i + 1}", "qtype": t, "text": q, "gold": g, "stale": s, "evidence_facts": e}
        for i, (t, q, g, s, e) in enumerate(questions)
    ], {"pending": pending, "paused": paused, "unknown": unknown}


def build_scenario(index: int) -> dict:
    rng = random.Random(SEED * 1000 + index)
    module = MODULES[index % len(MODULES)]
    others = [m for m in MODULES if m is not module]
    fact_items, questions, open_labels = build_facts(rng, module)
    record = []
    seq = 0
    for session in range(4):
        session_facts = [f for f in fact_items if f["session"] == session]
        turns = 5
        slots = rng.sample(range(turns * 4), len(session_facts)) if session < 3 else []
        for turn in range(turns):
            seq += 1
            topic = module if rng.random() < 0.4 else rng.choice(others)
            text = f"{topic['ko']} 쪽 다음 단계 진행해 줘"
            if session == 0 and turn == 0:
                text = f"{module['ko']} 결제 흐름의 timeout_ms를 조정한다"
            elif session == 3 and turn == 1:
                text = f"대기: {open_labels['pending']} 결과를 다음 session에서 확인한다"
            elif session == 3 and turn == 2:
                text = f"보류: {open_labels['paused']}는 다른 작업 뒤에 재개한다"
            elif session == 3 and turn == 3:
                text = f"결과 미확인: {open_labels['unknown']}는 확인 전까지 완료로 말하지 않는다"
            record.append({"seq": seq, "session": session + 1, "kind": "user", "text": text})
            for call in range(4):
                seq += 1
                slot = turn * 4 + call
                fact = next((f for f, s in zip(session_facts, slots) if s == slot), None)
                tool = rng.choice(["shell", "test", "read", "edit"])
                topic = rng.choice(others) if fact is None else module
                path = f"src/{topic['en']}/{rng.choice(['mod', 'handler', 'config', 'store'])}.rs"
                if fact is not None:
                    tool = fact["tool"]
                    path = rng.choice(["config/app.toml", "ops/run.log", path])
                body = filler(rng, topic, rng.randint(200, 4000))
                if fact is not None:
                    lines = body.split("\n")
                    lines.insert(rng.randint(0, len(lines)), fact["text"])
                    body = "\n".join(lines)
                    fact["seq"] = seq
                record.append({"seq": seq, "session": session + 1, "kind": "tool", "tool": tool,
                               "args": {"path": path}, "result": body})
            seq += 1
            record.append({"seq": seq, "session": session + 1, "kind": "agent", "text": "이 단계를 마쳤다."})
    seq += 1
    record.append({"seq": seq, "session": 4, "kind": "user",
                   "text": f"{module['ko']} 쪽 마무리하자. 남은 실패 고쳐 줘"})
    for row in record:
        row["ts"] = f"{SESSION_DATES[row['session'] - 1]}T10:00:00Z"
    for q in questions:
        q["evidence_seq"] = [fact_items[i]["seq"] for i in q["evidence_facts"]]
        q["evidence_relation"] = [fact_items[i]["relation"] for i in q["evidence_facts"]]
    users = [r for r in record if r["kind"] == "user"]
    first, last = users[0], users[-1]
    open_items = [
        {"seq": r["seq"], "text": r["text"]}
        for r in users
        if r["text"].startswith(("대기:", "보류:", "결과 미확인:"))
    ]
    questions[0]["evidence_seq"] = [first["seq"]]
    questions[1]["evidence_seq"] = [first["seq"], last["seq"]]
    questions[2]["evidence_seq"] = [last["seq"]]
    questions[3]["evidence_seq"] = [last["seq"]]
    questions[4]["evidence_seq"] = [open_items[0]["seq"]]
    questions[5]["evidence_seq"] = [open_items[1]["seq"]]
    questions[6]["evidence_seq"] = [open_items[2]["seq"]]
    questions[7]["evidence_seq"] = [open_items[2]["seq"]]
    for q in questions:
        if q["evidence_seq"] and not q["evidence_relation"]:
            q["evidence_relation"] = ["fixed"] * len(q["evidence_seq"])
    order = list(range(len(questions)))
    rng.shuffle(order)
    return {"scenario_id": f"s{index + 1:02d}", "module": module["en"], "record": record,
            "questions": [questions[i] for i in order],
            "fixed": {"goal": [{"seq": first["seq"], "text": first["text"]},
                                 {"seq": last["seq"], "text": last["text"]}],
                      "open_items": open_items}}


# ---------------------------------------------------------------- judge

def judge_post(body: dict, key: str) -> dict | None:
    request = urllib.request.Request(JUDGE_ENDPOINT, data=json.dumps(body).encode(), method="POST", headers={
        "authorization": f"Bearer {key}", "content-type": "application/json"})
    for attempt in range(JUDGE_RETRIES):
        try:
            with urllib.request.urlopen(request, timeout=JUDGE_TIMEOUT_S) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code not in (429, 529) or attempt == JUDGE_RETRIES - 1:
                return None
            time.sleep(float(error.headers.get("retry-after") or 2))
        except (urllib.error.URLError, OSError, json.JSONDecodeError):
            if attempt == JUDGE_RETRIES - 1:
                return None
    return None


def judge_questions(scenario: dict) -> tuple[str, list[tuple[str, str]]]:
    record = scenario["record"]
    users = [r for r in record if r["kind"] == "user"]
    state = "Latest user request:\n" + users[-1]["text"] + "\n\nEarlier user requests (oldest first):\n" + \
        "\n".join("- " + u["text"] for u in users[-1 - BASE_TURNS:-1])
    head = "The user moves this chat to a fresh coding-agent session for the latest request. "
    questions = []
    for r in record:
        if r["kind"] != "tool":
            continue
        call = f"{r['tool']} {json.dumps(r['args'], ensure_ascii=False)}"
        questions.append((f"call_{r['seq']}_keep", head + "Should the new session see this tool call?\n\n" + call))
        questions.append((f"result_{r['seq']}_keep", head + "Should the new session see this tool result?\n\n"
                          + call + "\n\nResult:\n" + r["result"][:RESULT_LIMIT_CHARS]))
    return state, questions


def split_questions(state: str, questions: list) -> list[list]:
    chunks, chunk = [], []
    for question in questions:
        size = len(state.encode()) + sum(len(q[1].encode()) + 120 for q in chunk + [question])
        if chunk and size > JUDGE_MAX_BODY_BYTES:
            chunks.append(chunk)
            chunk = []
        chunk.append(question)
    if chunk:
        chunks.append(chunk)
    return chunks


def judge_all(scenario: dict, key: str) -> dict:
    """후보 전체를 64K 이하 요청으로 나눠 동시 8개까지 보내고 항목별 남길 확률을 돌려준다."""
    state, questions = judge_questions(scenario)
    chunks = split_questions(state, questions)

    def send(chunk: list) -> tuple[list, dict | None]:
        body = {"model": JUDGE_MODEL, "state": state,
                "questions": {qid: {"type": "noul", "instructions": text} for qid, text in chunk}}
        return chunk, judge_post(body, key)

    answers, failures = {}, 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=JUDGE_PARALLEL) as pool:
        for chunk, response in pool.map(send, chunks):
            if response is None:
                failures += 1
                continue
            for qid, _ in chunk:
                value = ((response.get("answers") or {}).get(qid) or {}).get("noul")
                if isinstance(value, (int, float)) and not math.isnan(value) and 0 <= value <= 1:
                    answers[qid] = value
    compact = []
    for r in scenario["record"]:
        if r["kind"] != "tool":
            continue
        values = [answers[f"{k}_{r['seq']}_keep"] for k in KEEP_KEYS if f"{k}_{r['seq']}_keep" in answers]
        if values:
            compact.append({"seq": r["seq"], "probability": max(values)})
    return {"compact": compact, "judge_calls": len(chunks), "judge_failures": failures,
            "judge_questions": len(questions)}


# ---------------------------------------------------------------- 패킷과 provider

def build_packet(packet_cmd: list, scenario_path: Path, scenario_id: str, condition: str,
                 fixed_file: Path | None) -> dict:
    args = packet_cmd + ["--scenarios", str(scenario_path), "--scenario-id", scenario_id, "--k", str(RRF_K),
                         "--budget", str(PACKET_BUDGET_TOKENS), "--condition", "rrf-only"]
    if fixed_file is not None:
        args += ["--fixed-file", str(fixed_file)]
    done = subprocess.run(args, capture_output=True, text=True, check=False, timeout=SESSION_TIMEOUT_S,
                          cwd=EXPERIMENT)
    if done.returncode != 0:
        raise RuntimeError(f"패킷 생성 실패: 종료 코드 {done.returncode}: {redact(done.stderr[-300:])}")
    return json.loads(done.stdout)


def summary_prompt(scenario: dict) -> str:
    return (
        "기록만 읽고 새 coding-agent session에 넘길 고정 구역을 만든다. "
        "첫 사용자 입력과 마지막 사용자 입력을 goal에 순서대로 넣고, 대기·보류·결과 미확인 입력을 open_items에 넣는다. "
        "각 항목의 seq와 원문 text를 보존한다. 도구 결과를 추측하거나 항목을 추가하지 마라. "
        'JSON 하나만 출력하라: {"goal":[{"seq":1,"text":"..."}],"open_items":[{"seq":2,"text":"..."}]}\n\n'
        + json.dumps(scenario["record"], ensure_ascii=False)
    )


def provider_text(stdout: str) -> tuple[str, int]:
    text, tools = "", 0
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") or {}
        kind = item.get("type", "")
        if event.get("type") == "item.completed" and kind == "agent_message":
            text = item.get("text", "")
        elif event.get("type") == "item.started" and kind not in ("agent_message", "reasoning"):
            tools += 1
    return text, tools


def token_usage(stdout: str, input_fallback: str, output_fallback: str) -> dict:
    input_tokens = output_tokens = 0
    for line in stdout.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        stack = [value]
        while stack:
            item = stack.pop()
            if isinstance(item, dict):
                usage = item.get("usage")
                if isinstance(usage, dict):
                    input_tokens = max(input_tokens, int(usage.get("input_tokens", 0) or 0))
                    output_tokens = max(output_tokens, int(usage.get("output_tokens", 0) or 0))
                stack.extend(item.values())
            elif isinstance(item, list):
                stack.extend(item)
    if input_tokens == 0:
        input_tokens = len(input_fallback) // 4
    if output_tokens == 0:
        output_tokens = len(output_fallback) // 4
    return {"input_tokens": input_tokens, "output_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens}


def build_summary(scenario: dict, call_number: int) -> dict:
    prompt = summary_prompt(scenario)
    done = subprocess.run(
        ["codex", "exec", "-m", SUMMARY_MODEL, "--ignore-user-config", "--ignore-rules",
         "--skip-git-repo-check", "--sandbox", "read-only", "--json", "-"],
        input=prompt, capture_output=True, text=True, check=False, timeout=SESSION_TIMEOUT_S, cwd=WORKDIR,
    )
    text, tools = provider_text(done.stdout)
    if done.returncode != 0 or tools:
        raise RuntimeError(f"요약 모델 실패: 종료 코드 {done.returncode}, 도구 호출 {tools}")
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end < start:
        raise RuntimeError("요약 모델이 JSON을 반환하지 않았다")
    fields = json.loads(text[start:end + 1])
    if not isinstance(fields.get("goal"), list) or not isinstance(fields.get("open_items"), list):
        raise RuntimeError("요약 모델 JSON의 고정 구역 필드가 없다")
    usage = token_usage(done.stdout, prompt, text)
    fields["summary_call"] = call_number
    fields["summary_input_tokens"] = usage["input_tokens"]
    fields["summary_output_tokens"] = usage["output_tokens"]
    fields["summary_total_tokens"] = usage["total_tokens"]
    return fields


def provider_args(provider: str) -> list:
    model = PROVIDER_MODELS[provider]
    if provider == "codex":
        return ["codex", "exec", "-m", model, "--ephemeral", "--ignore-user-config", "--ignore-rules",
                "--skip-git-repo-check", "--sandbox", "read-only", "--json", "-"]
    return ["claude", "-p", "--model", model, "--safe-mode", "--tools", "", "--no-session-persistence",
            "--output-format", "json"]


def ask(provider: str, prompt: str) -> dict:
    started = time.monotonic()
    try:
        done = subprocess.run(provider_args(provider), input=prompt, capture_output=True, text=True,
                              check=False, timeout=SESSION_TIMEOUT_S, cwd=WORKDIR)
        result = {"exit_code": done.returncode, "stdout": done.stdout, "stderr": done.stderr[-2000:]}
    except subprocess.TimeoutExpired:
        result = {"exit_code": None, "stdout": "", "stderr": "timeout"}
    result["elapsed_s"] = round(time.monotonic() - started, 3)
    return result


# ---------------------------------------------------------------- 사용량

def claude_usage() -> dict:
    done = subprocess.run(["claude", "-p", "/usage"], capture_output=True, text=True, check=False, cwd=WORKDIR)
    session = re.search(r"Current session:\s*(\d+(?:\.\d+)?)%", done.stdout)
    week = re.search(r"Current week \(all models\):\s*(\d+(?:\.\d+)?)%", done.stdout)
    return {"session": float(session.group(1)) if session else None,
            "week": float(week.group(1)) if week else None}


def codex_week_usage() -> float | None:
    """가장 최근 ~/.codex/sessions 기록의 주간(10080분) 사용률. 먼저 짧은 기록 하나를 남겨 값을 갱신한다."""
    subprocess.run(["codex", "exec", "-m", PROVIDER_MODELS["codex"], "--ignore-user-config", "--ignore-rules",
                    "--skip-git-repo-check", "--sandbox", "read-only", "-"], input="OK라고만 답하라",
                   capture_output=True, text=True, check=False, cwd=WORKDIR, timeout=SESSION_TIMEOUT_S)
    root = Path.home() / ".codex" / "sessions"
    files = sorted(root.rglob("*.jsonl"), key=lambda p: p.stat().st_mtime)
    for path in reversed(files[-5:]):
        for line in reversed(path.read_text(encoding="utf-8", errors="ignore").splitlines()):
            if '"rate_limits"' not in line:
                continue
            try:
                primary = json.loads(line)["payload"]["rate_limits"]["primary"]
            except (KeyError, TypeError, json.JSONDecodeError):
                continue
            if primary and primary.get("window_minutes") == 10080:
                return float(primary["used_percent"])
    return None


def usage_snapshot(label: str) -> dict:
    claude = claude_usage()
    return {"label": label, "ts_utc": now_utc(), "claude_session_percent": claude["session"],
            "claude_week_percent": claude["week"], "codex_week_percent": codex_week_usage()}


def load_env() -> dict:
    path = EXPERIMENT / "env.json"
    if not path.exists():
        return {"tools": {}}
    return json.loads(path.read_text(encoding="utf-8"))


def save_env(env: dict) -> None:
    (EXPERIMENT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def record_usage(label: str) -> dict:
    env = load_env()
    snap = usage_snapshot(label)
    env["usage"]["snapshots"].append(snap)
    if label == "start":
        env["usage"]["start"] = snap
    save_env(env)
    return snap


def guard_usage(label: str) -> None:
    """주간 사용률이 시작보다 5%p 오르면 멈추고, 5시간 창이 85%를 넘으면 초기화될 때까지 기다린다."""
    snap = record_usage(label)
    start = load_env()["usage"]["start"]
    for key in ("claude_week_percent", "codex_week_percent"):
        if snap[key] is None or start[key] is None:
            fail(f"사용률을 읽지 못했다: {key}", 3)
        if snap[key] - start[key] >= USAGE_LIMIT_PP:
            fail(f"{key}가 시작 {start[key]}%에서 {snap[key]}%로 {USAGE_LIMIT_PP}%p에 닿아 멈췄다", 3)
    waited = 0
    while snap["claude_session_percent"] is not None and snap["claude_session_percent"] > SESSION_WAIT_PERCENT:
        print(f"01-collect: Claude 5시간 창 {snap['claude_session_percent']}%, 초기화를 기다린다", file=sys.stderr)
        time.sleep(600)
        waited += 1
        snap = {**snap, "claude_session_percent": claude_usage()["session"]}
        if waited > 40:
            fail("Claude 5시간 창이 초기화되지 않아 멈췄다", 3)


# ---------------------------------------------------------------- 실행

def version(args: list) -> str:
    done = subprocess.run(args, capture_output=True, text=True, check=False)
    return done.stdout.strip() or done.stderr.strip()


def init_env(run_id: str, commit: str, packet_cmd: str) -> None:
    env = load_env()
    memory = subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True, check=False).stdout.strip()
    cpu = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True,
                         check=False).stdout.strip()
    env.update({
        "os": f"{platform.system()} {platform.release()}",
        "cpu": cpu or platform.machine(),
        "memory_gb": round(int(memory) / 2**30, 1) if memory.isdigit() else None,
        "run_date": datetime.date.today().isoformat(),
        "run_id": run_id,
        "commit": commit,
    })
    env["models"] = {"summary": SUMMARY_MODEL, "claude": PROVIDER_MODELS["claude"],
                      "codex": PROVIDER_MODELS["codex"]}
    env["tools"].update({"python": platform.python_version(), "codex": version(["codex", "--version"]),
                         "claude": version(["claude", "--version"]), "saturn_packet_cmd": packet_cmd})
    save_env(env)


def read_jsonl(path: Path) -> list:
    if not path.exists():
        return []
    with path.open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def run_lane(provider: str, scenario: dict, order: list, packets: dict, run_id: str, out, lock,
             done: set, state: dict) -> None:
    for condition in order:
        trial_id = f"{scenario['scenario_id']}-{provider}-{condition}"
        if trial_id in done:
            continue
        packet = packets[condition]
        context = packet["packet"]
        lines = [f"{q['qid']}. {q['text']}" for q in scenario["questions"]]
        prompt = f"{ANSWER_RULES}\n\n# 맥락\n\n{context}\n\n# 질문\n\n" + "\n".join(lines)
        for attempt in (1, 2):
            with lock:
                if state["provider_calls"] >= MAX_PROVIDER_CALLS:
                    state["stop"] = True
                    return
                state["provider_calls"] += 1
            answer = ask(provider, prompt)
            if answer["exit_code"] == 0:
                break
        row = {"run_id": run_id, "trial_id": trial_id, "condition": condition, "ts_utc": now_utc(),
               "scenario_id": scenario["scenario_id"], "provider": provider, "model": PROVIDER_MODELS[provider],
               "attempt": attempt, "packet_tokens": packet["tokens"],
               "prompt_tokens": len(prompt) // 4,
               "prompt_sha256": hashlib.sha256(prompt.encode("utf-8")).hexdigest(), **answer}
        with lock:
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
            state["failures"] = 0 if answer["exit_code"] == 0 else state["failures"] + 1
            if state["failures"] >= MAX_CONSECUTIVE_FAILURES:
                state["stop"] = True
        if state["stop"]:
            return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--limit", type=int, default=SCENARIO_COUNT)
    parser.add_argument("--resume")
    opts = parser.parse_args()
    for tool in ("claude", "codex", "cargo", "git"):
        if shutil.which(tool) is None:
            fail(f"{tool}이 PATH에 없다")
    WORKDIR.mkdir(parents=True, exist_ok=True)
    packet_cmd_text = os.environ.get("SATURN_PACKET_CMD", DEFAULT_PACKET_CMD)
    packet_cmd = shlex.split(packet_cmd_text)
    build = subprocess.run(["cargo", "build", "-q", "-p", "saturn-core", "--example", "packet"],
                           capture_output=True, text=True, check=False, cwd=EXPERIMENT)
    if build.returncode != 0:
        fail(f"패킷 예제 빌드 실패: {build.stderr[-300:]}")
    commit = version(["git", "-C", str(EXPERIMENT), "rev-parse", "--short=7", "HEAD"])
    run_id = opts.resume or f"{datetime.datetime.now(datetime.timezone.utc):%Y%m%dT%H%M%SZ}-{commit}"
    RAW.mkdir(parents=True, exist_ok=True)
    scenario_path = RAW / f"scenarios-{run_id}.jsonl"
    packets_path = RAW / f"packets-{run_id}.jsonl"
    sessions_path = RAW / f"sessions-{run_id}.jsonl"
    scenarios = [build_scenario(i) for i in range(SCENARIO_COUNT)]
    if not opts.resume:
        with scenario_path.open("w", encoding="utf-8", newline="\n") as out:
            for s in scenarios:
                out.write(json.dumps({"run_id": run_id, **s}, ensure_ascii=False) + "\n")
        init_env(run_id, commit, packet_cmd_text)
        env = load_env()
        env["summary_calls"] = 0
        env["provider_calls"] = 0
        save_env(env)
    order = list(scenarios)
    random.Random(SEED).shuffle(order)
    packets_done = {row["scenario_id"]: row for row in read_jsonl(packets_path)}
    done = {row["trial_id"] for row in read_jsonl(sessions_path)}
    lock, state = threading.Lock(), {"failures": 0, "stop": False, "provider_calls": 0,
                                     "summary_calls": 0}
    state["provider_calls"] = sum(1 for row in read_jsonl(sessions_path) for _ in [row])
    state["summary_calls"] = len(packets_done)
    with packets_path.open("a", encoding="utf-8", newline="\n") as packets_out, \
            sessions_path.open("a", encoding="utf-8", newline="\n") as sessions_out:
        for number, scenario in enumerate(order[:opts.limit], start=1):
            sid = scenario["scenario_id"]
            if sid not in packets_done:
                with lock:
                    if state["summary_calls"] >= MAX_SUMMARY_CALLS:
                        fail("요약 호출 상한에 닿아 중단했다", 3)
                    state["summary_calls"] += 1
                    summary_number = state["summary_calls"]
                summary = build_summary(scenario, summary_number)
                summary_path = WORKDIR / f"summary-{run_id}-{sid}.json"
                summary_path.write_text(json.dumps(summary, ensure_ascii=False), encoding="utf-8")
                fixed_path = WORKDIR / f"fixed-{run_id}-{sid}.json"
                fixed_path.write_text(json.dumps(scenario["fixed"], ensure_ascii=False), encoding="utf-8")
                built = {
                    "last-input": build_packet(packet_cmd, scenario_path, sid, "last-input", None),
                    "rule": build_packet(packet_cmd, scenario_path, sid, "rule", fixed_path),
                    "summary": build_packet(packet_cmd, scenario_path, sid, "summary", summary_path),
                }
                for condition in ("last-input", "rule", "summary"):
                    built[condition]["summary_total_tokens"] = summary["summary_total_tokens"] if condition == "summary" else 0
                row = {"run_id": run_id, "trial_id": f"{sid}-packets", "condition": "packets", "ts_utc": now_utc(),
                       "scenario_id": sid, "summary_calls": 1, "summary": summary, "packets": built}
                packets_out.write(json.dumps(row, ensure_ascii=False) + "\n")
                packets_out.flush()
                packets_done[sid] = row
            rng = random.Random(f"{SEED}-{sid}")
            threads = []
            for provider in PROVIDERS:
                lane_order = list(CONDITIONS)
                rng.shuffle(lane_order)
                threads.append(threading.Thread(target=run_lane, args=(
                    provider, scenario, lane_order, packets_done[sid]["packets"], run_id,
                    sessions_out, lock, done, state)))
            for t in threads:
                t.start()
            for t in threads:
                t.join()
            if state["stop"]:
                fail("provider 호출 상한 또는 연속 실패로 중단했다. 원인을 고친 뒤 새 실행 id로 다시 수집한다", 3)
            print(f"01-collect: {number}/{min(opts.limit, SCENARIO_COUNT)} {sid} 완료", flush=True)
    env = load_env()
    env["provider_calls"] = state["provider_calls"]
    env["summary_calls"] = state["summary_calls"]
    save_env(env)
    print(run_id)


if __name__ == "__main__":
    main()
