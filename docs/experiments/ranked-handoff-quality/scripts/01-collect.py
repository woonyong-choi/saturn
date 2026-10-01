"""합성 시나리오로 세 조건의 패킷을 만들고 새 provider session의 답을 수집한다.

사용: python3 scripts/01-collect.py (실험 폴더에서 실행)
조건이 갖춰지지 않으면 원인 한 줄을 쓰고 종료 코드 2로 끝난다.
"""
import datetime
import hashlib
import json
import os
import platform
import random
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

SEED = 117
SCENARIO_COUNT = 24
TOP_N = 10
RRF_K = 60
PACKET_BUDGET_TOKENS = 8000
SESSION_TIMEOUT_S = 300
MAX_CONSECUTIVE_FAILURES = 5
CONDITIONS = ["judge-only", "rrf-only", "rrf-judge"]
PROVIDERS = ["codex", "claude"]
EXPERIMENT = Path(__file__).resolve().parent.parent
RAW = EXPERIMENT / "data" / "raw"

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


def fail(reason: str) -> None:
    print(f"01-collect: {reason}", file=sys.stderr)
    sys.exit(2)


def check_preconditions() -> list[str]:
    if shutil.which("gh") is None:
        fail("gh가 없어 #126 머지 여부를 확인할 수 없다")
    state = subprocess.run(
        ["gh", "issue", "view", "126", "--json", "state", "-q", ".state"],
        capture_output=True, text=True, check=False,
    ).stdout.strip()
    if state != "CLOSED":
        fail(f"#126(후보 순위와 패킷 구역 규칙 구현)이 머지되지 않았다: {state or '확인 실패'}")
    if not os.environ.get("SATURN_JUDGE_KEY"):
        fail("SATURN_JUDGE_KEY에 Jev 키가 없다")
    for tool in PROVIDERS:
        if shutil.which(tool) is None:
            fail(f"provider 실행 파일 {tool}이 PATH에 없다")
    packet_cmd = os.environ.get("SATURN_PACKET_CMD", "")
    if not packet_cmd:
        fail("SATURN_PACKET_CMD에 패킷 생성 명령이 없다")
    return shlex.split(packet_cmd)


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


def build_packet(packet_cmd: list[str], scenario_path: Path, scenario_id: str, condition: str) -> dict:
    args = packet_cmd + ["--scenarios", str(scenario_path), "--scenario-id", scenario_id, "--condition", condition, "--top-n", str(TOP_N),
                         "--k", str(RRF_K), "--budget", str(PACKET_BUDGET_TOKENS)]
    done = subprocess.run(args, capture_output=True, text=True, check=False, timeout=SESSION_TIMEOUT_S)
    if done.returncode != 0:
        raise RuntimeError(f"패킷 생성 실패: 종료 코드 {done.returncode}")
    return json.loads(done.stdout)


def provider_args(provider: str) -> list[str]:
    if provider == "codex":
        return ["codex", "exec", "--ephemeral", "--ignore-user-config", "--ignore-rules",
                "--skip-git-repo-check", "--sandbox", "read-only", "--json", "-"]
    return ["claude", "-p", "--safe-mode", "--tools", "", "--no-session-persistence",
            "--output-format", "json"]


def ask(provider: str, prompt: str) -> dict:
    started = time.monotonic()
    try:
        done = subprocess.run(provider_args(provider), input=prompt, capture_output=True, text=True,
                              check=False, timeout=SESSION_TIMEOUT_S, cwd="/")
        result = {"exit_code": done.returncode, "stdout": done.stdout, "stderr": done.stderr[-2000:]}
    except subprocess.TimeoutExpired:
        result = {"exit_code": None, "stdout": "", "stderr": "timeout"}
    result["elapsed_s"] = round(time.monotonic() - started, 3)
    return result


def version(args: list[str]) -> str:
    done = subprocess.run(args, capture_output=True, text=True, check=False)
    return done.stdout.strip() or done.stderr.strip()


def write_env(run_id: str, commit: str) -> None:
    env = json.loads((EXPERIMENT / "env.json").read_text(encoding="utf-8"))
    env.update({
        "os": f"{platform.system()} {platform.release()}",
        "cpu": platform.processor() or platform.machine(),
        "memory_gb": round(os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30, 1),
        "run_date": datetime.date.today().isoformat(),
        "run_id": run_id,
        "commit": commit,
    })
    env["models"].update({key: os.environ.get(f"{key.upper()}_MODEL", "provider 기본값")
                          for key in ("judge", "codex", "claude")})
    env["tools"].update({"python": platform.python_version(), "codex": version(["codex", "--version"]),
                         "claude": version(["claude", "--version"]),
                         "saturn_packet_cmd": os.environ["SATURN_PACKET_CMD"]})
    (EXPERIMENT / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    packet_cmd = check_preconditions()
    commit = version(["git", "rev-parse", "--short=7", "HEAD"])
    run_id = f"{datetime.datetime.now(datetime.timezone.utc):%Y%m%dT%H%M%SZ}-{commit}"
    RAW.mkdir(parents=True, exist_ok=True)
    scenarios = [build_scenario(i) for i in range(SCENARIO_COUNT)]
    scenario_path = RAW / f"scenarios-{run_id}.jsonl"
    with scenario_path.open("w", encoding="utf-8") as out:
        for s in scenarios:
            out.write(json.dumps({"run_id": run_id, **s}, ensure_ascii=False) + "\n")
    jobs = [(s, p, c) for s in scenarios for p in PROVIDERS for c in CONDITIONS]
    random.Random(SEED).shuffle(jobs)
    write_env(run_id, commit)
    failures = 0
    with (RAW / f"sessions-{run_id}.jsonl").open("w", encoding="utf-8") as out:
        for scenario, provider, condition in jobs:
            packet = build_packet(packet_cmd, scenario_path, scenario["scenario_id"], condition)
            lines = [f"{q['qid']}. {q['text']}" for q in scenario["questions"]]
            prompt = f"{ANSWER_RULES}\n\n# 맥락\n\n{packet['packet']}\n\n# 질문\n\n" + "\n".join(lines)
            for attempt in (1, 2):
                answer = ask(provider, prompt)
                if answer["exit_code"] == 0:
                    break
            failures = 0 if answer["exit_code"] == 0 else failures + 1
            out.write(json.dumps({
                "run_id": run_id,
                "trial_id": f"{scenario['scenario_id']}-{provider}-{condition}",
                "condition": condition,
                "ts_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
                "scenario_id": scenario["scenario_id"],
                "provider": provider,
                "attempt": attempt,
                "packet_tokens": len(packet["packet"]) // 4,
                "packet_items": packet.get("included", []),
                "rrf_order": packet.get("rrf_order", []),
                "judge_calls": packet.get("judge_calls"),
                "judge_failures": packet.get("judge_failures"),
                "prompt_sha256": hashlib.sha256(prompt.encode("utf-8")).hexdigest(),
                **answer,
            }, ensure_ascii=False) + "\n")
            out.flush()
            if failures >= MAX_CONSECUTIVE_FAILURES:
                fail(f"연속 {failures}개 세션 실패로 중단했다. 원인을 고친 뒤 새 실행 id로 다시 수집한다")
    print(run_id)


if __name__ == "__main__":
    main()
