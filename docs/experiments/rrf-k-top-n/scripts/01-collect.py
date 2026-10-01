"""후보 세트를 만들고 judge 전체 판단과 채널 값을 수집한다.

원문은 저장소 밖 PRIVATE_DIR에, 원문 없는 후보별 수치는 data/raw/에 쓴다.
"""
import hashlib
import json
import math
import os
import platform
import random
import re
import subprocess
import sys
import time
import unicodedata
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
RAW_DIR = HERE / "data" / "raw"
PRIVATE_DIR = Path(os.environ.get(
    "RRF_PRIVATE_DIR",
    Path.home() / "workspace/woon/.local/orchestration/saturn-experiments/rrf-k-top-n/raw",
))
LOG_ROOT = Path(os.environ.get("RRF_LOG_ROOT", Path.home() / ".claude/projects"))
SESSION_DIR = re.compile(r"^-Users-[^-]+-(workspace-oss|orca-workspaces)-saturn(-[a-z0-9]+)*$", re.IGNORECASE)
ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-1.13.0"
QUESTION_VERSION = "compact-exp@1"
SEED = 116
MIN_CANDIDATES = 50
MAX_CANDIDATES = 150
MAX_POINTS_PER_SESSION = 10
MIN_SETS = 40
MAX_SETS = 120
MIN_KEPT = 400
REPLICA_SETS = 5
KEEP_THRESHOLD = 0.5
MAX_BODY_BYTES = 64_000
TEXT_LIMIT = 600
BM25_K1 = 1.2
BM25_B = 0.75
RETRIES = 3
TIMEOUT_S = 30
BASE_TURNS = 3

PATH_LIKE = re.compile(r"(?:[\w.@-]+/)+[\w.@-]+|[\w@-]+\.[A-Za-z][A-Za-z0-9]{0,5}\b")
ABS_PATH = re.compile(r"(?<![\w.])/(?:[\w.@-]+/)*([\w.@-]+)")
SECRET = re.compile(r"(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|Bearer\s+\S+)")
REPO_PREFIX = re.compile(r"^.*?/saturn(?:\.wt/[^/]+)?/")
CAMEL = re.compile(r"[a-z]+|[A-Z][a-z]*|[A-Z]+(?![a-z])")


def char_kind(ch):
    code = ord(ch)
    if 0xAC00 <= code <= 0xD7A3 or 0x1100 <= code <= 0x11FF or 0x3130 <= code <= 0x318F:
        return "hangul"
    if 0x4E00 <= code <= 0x9FFF or 0x3040 <= code <= 0x30FF:
        return "cjk"
    if ch.isdigit():
        return "digit"
    if ch.isalpha():
        return "latin"
    return "sep"


def word_pieces(text):
    """맥락 고르기의 단어 조각 규칙. 용어 카탈로그 짝은 없다."""
    text = unicodedata.normalize("NFC", text)
    runs, kind, buf = [], None, []
    for ch in text:
        current = char_kind(ch)
        if current != kind and buf:
            runs.append((kind, "".join(buf)))
            buf = []
        kind = current
        buf.append(ch)
    if buf:
        runs.append((kind, "".join(buf)))
    pieces = []
    for kind, run in runs:
        if kind in ("hangul", "cjk"):
            pieces += [run[i:i + 2] for i in range(len(run) - 1)] or [run]
        elif kind == "latin":
            pieces += [w.lower() for w in CAMEL.findall(run)]
        elif kind == "digit":
            pieces.append(run)
    return pieces


def self_test():
    a, b = set(word_pieces("로그인 실패")), set(word_pieces("로그인실패를"))
    assert {"로그", "그인", "실패"} <= a & b
    assert {"auth", "login"} <= set(word_pieces("authLogin.rs")) & set(word_pieces("auth_login"))
    assert word_pieces(unicodedata.normalize("NFD", "로그인.md"))[:2] == word_pieces("로그인")


def normalize_path(path):
    path = REPO_PREFIX.sub("", path.strip().strip("'\"`"))
    return path[2:] if path.startswith("./") else path


def paths_in(tool_input):
    found = set()
    for key in ("file_path", "path", "notebook_path"):
        if isinstance(tool_input.get(key), str):
            found.add(normalize_path(tool_input[key]))
    for key in ("command", "pattern", "glob"):
        if isinstance(tool_input.get(key), str):
            found |= {normalize_path(p) for p in PATH_LIKE.findall(tool_input[key])}
    return found


def sanitize(text):
    key = os.environ.get("SATURN_JUDGE_KEY", "")
    if key:
        text = text.replace(key, "[secret]")
    text = SECRET.sub("[secret]", text)
    return ABS_PATH.sub(lambda m: "[abs]/" + m.group(1), text)


def clip(text):
    data = text.encode()
    return data[:TEXT_LIMIT].decode(errors="ignore") if len(data) > TEXT_LIMIT else text


def result_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(x.get("text", "") for x in content if isinstance(x, dict))
    return ""


def read_session(path):
    """메인 session의 사용자 입력과 도구 호출을 기록 순서대로 읽는다."""
    inputs, calls, by_id = [], [], {}
    for line in path.open(encoding="utf-8"):
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("isSidechain") or row.get("type") not in ("user", "assistant"):
            continue
        content = (row.get("message") or {}).get("content")
        if row["type"] == "user" and isinstance(content, str) and not content.startswith("<"):
            inputs.append({"text": content, "after_calls": len(calls)})
            continue
        for block in content if isinstance(content, list) else []:
            if block.get("type") == "tool_use":
                call = {"id": block["id"], "name": block.get("name", ""),
                        "input": block.get("input") or {}, "result": None, "turn": len(inputs)}
                by_id[call["id"]] = call
                calls.append(call)
            elif block.get("type") == "tool_result" and block.get("tool_use_id") in by_id:
                by_id[block["tool_use_id"]]["result"] = result_text(block.get("content"))
            elif block.get("type") == "text" and row["type"] == "user":
                if not block.get("text", "").startswith("<"):
                    inputs.append({"text": block["text"], "after_calls": len(calls)})
    return inputs, calls


def input_points(rng):
    points = []
    for folder in sorted(LOG_ROOT.iterdir()):
        if not SESSION_DIR.search(folder.name):
            continue
        for path in sorted(folder.glob("*.jsonl")):
            inputs, calls = read_session(path)
            eligible = []
            for index, entry in enumerate(inputs):
                prior = [c for c in calls[:entry["after_calls"]] if c["result"] is not None]
                if len(prior) >= MIN_CANDIDATES:
                    eligible.append((path, index, inputs, prior[-MAX_CANDIDATES:]))
            rng.shuffle(eligible)
            points += eligible[:MAX_POINTS_PER_SESSION]
    rng.shuffle(points)
    return points


def build_set(path, index, inputs, candidates):
    last = inputs[index]["text"]
    turn = index
    base = {normalize_path(p) for p in PATH_LIKE.findall(last)}
    for call in candidates:
        if call["turn"] > turn - BASE_TURNS:
            base |= paths_in(call["input"])
    texts = [call["name"] + " " + json.dumps(call["input"], ensure_ascii=False) + "\n" + (call["result"] or "")
             for call in candidates]
    docs = [word_pieces(clip(sanitize(t))) for t in texts]
    query = word_pieces(last)
    avg_len = sum(map(len, docs)) / len(docs) or 1
    df = Counter(p for d in docs for p in set(d))
    rows = []
    for record_no, (call, doc) in enumerate(zip(candidates, docs), start=1):
        tf = Counter(doc)
        bm25 = 0.0
        for piece in set(query):
            if tf[piece]:
                idf = math.log(1 + (len(docs) - df[piece] + 0.5) / (df[piece] + 0.5))
                norm = tf[piece] + BM25_K1 * (1 - BM25_B + BM25_B * len(doc) / avg_len)
                bm25 += idf * tf[piece] * (BM25_K1 + 1) / norm
        paths = paths_in(call["input"])
        rows.append({"record_no": record_no, "has_paths": bool(paths),
                     "file_overlap": len(paths & base), "bm25": round(bm25, 6)})
    state = "Latest user request:\n" + sanitize(last) + "\n\nEarlier user requests (oldest first):\n" + \
        "\n".join("- " + clip(sanitize(e["text"])) for e in inputs[max(0, index - BASE_TURNS):index])
    set_id = hashlib.sha256(f"{path.name}:{index}".encode()).hexdigest()[:12]
    return set_id, state, candidates, rows


def questions_for(candidates, rng):
    order = list(range(len(candidates)))
    rng.shuffle(order)
    questions = []
    for i in order:
        call = candidates[i]
        head = "The user moves this chat to a fresh coding-agent session for the latest request. "
        call_text = clip(sanitize(call["name"] + " " + json.dumps(call["input"], ensure_ascii=False)))
        questions.append((f"call_{i + 1}_keep", head + "Should the new session see this tool call?\n\n" + call_text))
        questions.append((f"result_{i + 1}_keep", head + "Should the new session see this tool result?\n\n"
                          + call_text + "\n\nResult:\n" + clip(sanitize(call["result"] or ""))))
    return questions


def post(body, key):
    request = urllib.request.Request(ENDPOINT, data=json.dumps(body).encode(), method="POST", headers={
        "authorization": f"Bearer {key}", "content-type": "application/json"})
    for attempt in range(RETRIES):
        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT_S) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code not in (429, 529) or attempt == RETRIES - 1:
                return None
            time.sleep(float(error.headers.get("retry-after") or 2))
        except (urllib.error.URLError, TimeoutError):
            if attempt == RETRIES - 1:
                return None
    return None


def judge(state, questions, key):
    """질문을 64K 이하 요청으로 나눠 보내고 질문별 P(yes)나 오류 상태를 돌려준다."""
    answers, responses, chunk = {}, [], []
    chunks = []
    for question in questions:
        size = len(state.encode()) + sum(len(q[1].encode()) + 120 for q in chunk + [question])
        if chunk and size > MAX_BODY_BYTES:
            chunks.append(chunk)
            chunk = []
        chunk.append(question)
    chunks.append(chunk)
    for chunk in chunks:
        body = {"model": MODEL, "state": state,
                "questions": {qid: {"type": "noul", "instructions": text} for qid, text in chunk}}
        response = post(body, key)
        responses.append(response)
        if response is None:
            return "failed", answers, responses
        for qid, _ in chunk:
            value = ((response.get("answers") or {}).get(qid) or {}).get("noul")
            valid = isinstance(value, (int, float)) and not math.isnan(value) and 0 <= value <= 1
            answers[qid] = value if valid else None
    status = "ok" if all(v is not None for v in answers.values()) else "invalid"
    return status, answers, responses


def set_id_session(path):
    return hashlib.sha256(path.stem.encode()).hexdigest()[:12]


def git_commit():
    return subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], capture_output=True,
                          text=True, cwd=HERE, check=True).stdout.strip()


def write_env(run_id, commit):
    env = json.loads((HERE / "env.json").read_text())
    mem = subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True).stdout.strip()
    cpu = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip()
    env.update({"os": f"{platform.system()} {platform.release()}", "cpu": cpu or platform.machine(),
                "memory_bytes": int(mem) if mem.isdigit() else None,
                "tools": {"python": platform.python_version()}, "run_id": run_id,
                "run_date": run_id[:8], "commit": commit})
    (HERE / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n")


def main():
    key = os.environ.get("SATURN_JUDGE_KEY")
    if not key:
        print("01-collect: SATURN_JUDGE_KEY(Jev 키)가 없어 judge 전체 판단을 만들 수 없다", file=sys.stderr)
        return 2
    self_test()
    commit = git_commit()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + commit
    RAW_DIR.mkdir(parents=True, exist_ok=True)
    PRIVATE_DIR.mkdir(parents=True, exist_ok=True)
    write_env(run_id, commit)
    rng = random.Random(SEED)
    attempted = ok_sets = kept_total = 0
    public = (RAW_DIR / f"claude-{run_id}.jsonl").open("w", encoding="utf-8", newline="\n")
    private = (PRIVATE_DIR / f"claude-{run_id}.jsonl").open("w", encoding="utf-8", newline="\n")
    for path, index, inputs, candidates in input_points(rng):
        if attempted >= MAX_SETS or (ok_sets >= MIN_SETS and kept_total >= MIN_KEPT):
            break
        attempted += 1
        set_id, state, candidates, rows = build_set(path, index, inputs, candidates)
        questions = questions_for(candidates, rng)
        repeats = 2 if ok_sets < REPLICA_SETS else 1
        for repeat in range(1, repeats + 1):
            ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
            status, answers, responses = judge(state, questions, key)
            private.write(json.dumps({"run_id": run_id, "trial_id": set_id, "repeat": repeat, "ts_utc": ts,
                                      "question_version": QUESTION_VERSION, "state": state,
                                      "questions": questions, "responses": responses},
                                     ensure_ascii=False) + "\n")
            for row in rows:
                i = row["record_no"]
                public.write(json.dumps({
                    "run_id": run_id, "trial_id": set_id, "condition": "full-judge", "repeat": repeat,
                    "ts_utc": ts, "candidate_id": f"{set_id}-{i:03d}", "session_id": set_id_session(path),
                    "status": status, "p_call": answers.get(f"call_{i}_keep"),
                    "p_result": answers.get(f"result_{i}_keep"), **row}) + "\n")
            if status != "ok":
                break
            if repeat == 1:
                ok_sets += 1
                kept_total += sum(max(answers[f"call_{r['record_no']}_keep"],
                                      answers[f"result_{r['record_no']}_keep"]) >= KEEP_THRESHOLD for r in rows)
    public.close()
    private.close()
    print(f"01-collect: run {run_id}, 시도 {attempted}세트, 정상 {ok_sets}세트, kept {kept_total}개")
    return 0


if __name__ == "__main__":
    sys.exit(main())
