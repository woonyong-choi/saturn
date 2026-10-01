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

SEED = 218
SCENARIO_COUNT = 48
RRF_K = 60
PACKET_BUDGET_TOKENS = 8000
SESSION_TIMEOUT_S = 300
MAX_CONSECUTIVE_FAILURES = 5
CONDITIONS = ["judge-all", "rrf-fallback", "no-packet"]
PROVIDERS = ["claude", "codex"]
PROVIDER_MODELS = {"claude": "claude-opus-5-5", "codex": "gpt-6-sol"}
JUDGE_ENDPOINT = "https://api.typesafe.ai/v1/systemone"
JUDGE_MODEL = "jev-1.13.0"
JUDGE_MAX_BODY_BYTES = 64_000
JUDGE_PARALLEL = 8
JUDGE_RETRIES = 3
JUDGE_TIMEOUT_S = 30
RESULT_LIMIT_CHARS = 4000
BASE_TURNS = 3
KEEP_KEYS = ("call", "result")
USAGE_LIMIT_PP = 4.0
SESSION_WAIT_PERCENT = 85
USAGE_CHECK_EVERY = 6
EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"
WORKDIR = Path(os.environ.get(
    "HANDOFF_WORKDIR",
    Path.home() / "workspace/woon/.local/orchestration/saturn-experiments/handoff-packet-quality-v2",
))
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


def build_facts(rng: random.Random, module: dict) -> tuple[list[dict], list[dict]]:
    relations = [RELATIONS[i % 3] for i in range(9)]
    rng.shuffle(relations)
    timeout = rng.randint(1200, 9800)
    retries = rng.randint(2, 9)
    fail_a, fail_b = rng.randint(2, 19), rng.randint(2, 19)
    code_a, code_b = f"E{rng.randint(100, 499)}", f"E{rng.randint(500, 999)}"
    old_limit, new_limit = rng.randint(10, 40), rng.randint(41, 90)
    old_ttl, new_ttl = rng.randint(30, 120), rng.randint(121, 600)
    cache_off = rng.choice([1, 2])
    first_pick = rng.choice(["rate-limit", "index"])
    facts = [
        (0, 1, "read", f"{term(module, relations[0])} timeout_ms = {timeout}"),
        (1, 1, "read", f"{term(module, relations[1])} retry_count = {retries}"),
        (2, 0, "test", f"{term(module, relations[2])} tests: {fail_a} failed"),
        (3, 2, "test", f"{term(module, relations[3])} tests: {fail_b} failed"),
        (4, 0, "shell", f"{term(module, relations[4])} error code {code_a}"),
        (5, 2, "shell", f"{term(module, relations[5])} error code {code_b}"),
        (6, cache_off, "edit", f"{term(module, relations[6])} cache disabled"),
        (7, 0 if first_pick == "rate-limit" else 1, "edit", f"{term(module, relations[7])} rate-limit added"),
        (7, 1 if first_pick == "rate-limit" else 0, "edit", f"{term(module, relations[7])} index added"),
        (8, 0, "edit", f"{term(module, relations[8])} max_items = {old_limit}"),
        (8, 2, "edit", f"{term(module, relations[8])} max_items = {new_limit}"),
        (8, 0, "read", f"{module['ko']} session_ttl_s = {old_ttl}"),
        (8, 2, "read", f"{module['ko']} session_ttl_s = {new_ttl}"),
    ]
    fact_items = [
        {"fact": i, "session": s, "tool": tool, "text": text, "relation": relation_of(module, text)}
        for i, (_, s, tool, text) in enumerate(facts)
    ]
    ko = module["ko"]
    questions = [
        ("extract", f"{ko} 요청 타임아웃은 몇 ms로 설정되어 있나", [str(timeout)], [], [0]),
        ("extract", f"{ko} 재시도 횟수는 몇 번인가", [str(retries)], [], [1]),
        ("multi-session", f"{SESSION_DATES[0]}과 {SESSION_DATES[2]}에 실패한 {ko} 테스트 수의 합은", [str(fail_a + fail_b)], [], [2, 3]),
        ("multi-session", f"{ko} 작업 중 나온 오류 코드 두 개는", [code_a, code_b], [], [4, 5]),
        ("temporal", f"{ko} 캐시를 끈 날짜는", [SESSION_DATES[cache_off]], [], [6]),
        ("temporal", f"{ko}에 rate-limit과 index 중 먼저 추가한 것은", [first_pick], [], [7, 8]),
        ("update", f"{ko} max_items의 현재 값은", [str(new_limit)], [str(old_limit)], [9, 10]),
        ("update", f"{ko} session_ttl_s의 현재 값은", [str(new_ttl)], [str(old_ttl)], [11, 12]),
        ("abstain", f"{ko} 데이터베이스 연결 풀 크기는", [], [], []),
        ("abstain", f"{ko} 모듈을 배포한 서버 이름은", [], [], []),
    ]
    return fact_items, [
        {"qid": f"q{i + 1}", "qtype": t, "text": q, "gold": g, "stale": s, "evidence_facts": e}
        for i, (t, q, g, s, e) in enumerate(questions)
    ]


def build_scenario(index: int) -> dict:
    rng = random.Random(SEED * 1000 + index)
    module = MODULES[index % len(MODULES)]
    others = [m for m in MODULES if m is not module]
    fact_items, questions = build_facts(rng, module)
    record = []
    seq = 0
    for session in range(4):
        session_facts = [f for f in fact_items if f["session"] == session]
        turns = 8
        slots = rng.sample(range(turns * 4), len(session_facts)) if session < 3 else []
        for turn in range(turns):
            seq += 1
            topic = module if rng.random() < 0.4 else rng.choice(others)
            record.append({"seq": seq, "session": session + 1, "kind": "user",
                           "text": f"{topic['ko']} 쪽 다음 단계 진행해 줘"})
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
    order = list(range(len(questions)))
    rng.shuffle(order)
    return {"scenario_id": f"s{index + 1:02d}", "module": module["en"], "record": record,
            "questions": [questions[i] for i in order]}


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
                 judgments: Path | None) -> dict:
    args = packet_cmd + ["--scenarios", str(scenario_path), "--scenario-id", scenario_id, "--k", str(RRF_K),
                         "--budget", str(PACKET_BUDGET_TOKENS),
                         "--condition", "judge-only" if condition == "judge-all" else "rrf-only"]
    if judgments is not None:
        args += ["--judgments", str(judgments)]
    done = subprocess.run(args, capture_output=True, text=True, check=False, timeout=SESSION_TIMEOUT_S,
                          cwd=EXPERIMENT)
    if done.returncode != 0:
        raise RuntimeError(f"패킷 생성 실패: 종료 코드 {done.returncode}: {redact(done.stderr[-300:])}")
    return json.loads(done.stdout)


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
    return json.loads((EXPERIMENT / "env.json").read_text(encoding="utf-8"))


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
    env["models"] = {"judge": JUDGE_MODEL, "claude": PROVIDER_MODELS["claude"], "codex": PROVIDER_MODELS["codex"]}
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
        packet = packets.get(condition)
        if condition == "no-packet":
            last = [r for r in scenario["record"] if r["kind"] == "user"][-1]["text"]
            context, tokens = f"사용자의 마지막 입력: {last}", len(last) // 4
        else:
            context, tokens = packet["packet"], len(packet["packet"]) // 4
        lines = [f"{q['qid']}. {q['text']}" for q in scenario["questions"]]
        prompt = f"{ANSWER_RULES}\n\n# 맥락\n\n{context}\n\n# 질문\n\n" + "\n".join(lines)
        for attempt in (1, 2):
            answer = ask(provider, prompt)
            if answer["exit_code"] == 0:
                break
        row = {"run_id": run_id, "trial_id": trial_id, "condition": condition, "ts_utc": now_utc(),
               "scenario_id": scenario["scenario_id"], "provider": provider, "model": PROVIDER_MODELS[provider],
               "attempt": attempt, "packet_tokens": tokens,
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
    key = os.environ.get("SATURN_JUDGE_KEY")
    if not key:
        fail("SATURN_JUDGE_KEY에 Jev 키가 없다")
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
        record_usage("start")
    order = list(scenarios)
    random.Random(SEED).shuffle(order)
    packets_done = {row["scenario_id"]: row for row in read_jsonl(packets_path)}
    done = {row["trial_id"] for row in read_jsonl(sessions_path)}
    lock, state = threading.Lock(), {"failures": 0, "stop": False}
    with packets_path.open("a", encoding="utf-8", newline="\n") as packets_out, \
            sessions_path.open("a", encoding="utf-8", newline="\n") as sessions_out:
        for number, scenario in enumerate(order[:opts.limit], start=1):
            sid = scenario["scenario_id"]
            if number > 1 and (number - 1) % USAGE_CHECK_EVERY == 0:
                guard_usage(f"before-scenario-{number}")
            if sid not in packets_done:
                judged = judge_all(scenario, key)
                jpath = WORKDIR / f"judgments-{run_id}-{sid}.json"
                jpath.write_text(json.dumps({"compact": judged["compact"], "constraints": []}), encoding="utf-8")
                built = {}
                for condition in ("judge-all", "rrf-fallback"):
                    built[condition] = build_packet(packet_cmd, scenario_path, sid, condition,
                                                    jpath if condition == "judge-all" else None)
                row = {"run_id": run_id, "trial_id": f"{sid}-packets", "condition": "packets", "ts_utc": now_utc(),
                       "scenario_id": sid, "judge_calls": judged["judge_calls"],
                       "judge_failures": judged["judge_failures"], "judge_questions": judged["judge_questions"],
                       "judged": len(judged["compact"]), "judgments": judged["compact"], "packets": built}
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
                fail("연속 5개 세션이 실패해 중단했다. 원인을 고친 뒤 새 실행 id로 다시 수집한다")
            print(f"01-collect: {number}/{min(opts.limit, SCENARIO_COUNT)} {sid} 완료", flush=True)
    label = "after-pilot" if opts.limit < SCENARIO_COUNT else "end"
    record_usage(label)
    print(run_id)


if __name__ == "__main__":
    main()
