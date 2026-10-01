"""Saturn 저장소 고정 커밋으로 말뭉치와 오타 질의를 만들고 조건별 BM25 순위를 기록한다.

명령:
    collect  말뭉치, 질의, 순위를 data/raw/에 새 실행 id로 쓰고 env.json과 SHA256SUMS를 갱신한다.
    verify   가장 최근 실행의 raw 파일을 같은 입력으로 다시 계산한 값과 비교한다.
    plan     말뭉치 문서 수와 질의 후보 수만 출력한다. 순위는 계산하지 않는다.
"""

import datetime
import hashlib
import json
import math
import os
import platform
import random
import re
import subprocess
import sys
import unicodedata
from collections import Counter, defaultdict

CORPUS_COMMIT = "1279d4047f4eb93bb297da27ef7d6b4b7fd65acf"
SEED = 118
QUERIES_PER_LANG = 800
MAX_QUERIES_PER_DOC = 8
MAX_GOLD_DOCS = 5
CODE_CHUNK_LINES = 40
TOP_K = 10
BM25_K1 = 1.2
BM25_B = 0.75

KO_WINDOW_WORDS = (2, 3, 4)
EN_WINDOW_WORDS = (2, 3)
KO_TYPOS = ("jamo-sub", "jamo-del", "space-drop")
EN_TYPOS = ("letter-sub", "letter-ins", "letter-del", "letter-swap", "space-drop")

# 조건 이름: (한글 조각, 영문 조각). 앞 셋이 확인 분석, 나머지는 탐색 분석이다.
CONDITIONS = {
    "base": (("syl2",), ("word",)),
    "ko-jamo3": (("jamo3",), ("word",)),
    "en-char4": (("syl2",), ("char4",)),
    "ko-jamo2": (("jamo2",), ("word",)),
    "ko-jamo4": (("jamo4",), ("word",)),
    "ko-union": (("syl2", "jamo3"), ("word",)),
    "en-union": (("syl2",), ("word", "char4")),
}

RUST_KEYWORDS = frozenset(
    "as async await break const continue crate dyn else enum extern false fn for if impl in "
    "let loop match mod move mut pub ref return self Self static struct super trait true type "
    "unsafe use where while".split()
)
COMPAT_CHOSEONG = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ"
HANGUL_WORD = re.compile(r"^[가-힣]+$")
IDENT_WORD = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
STRING_LITERAL = re.compile(r'r#*"[^"]*"#*|"(?:\\.|[^"\\])*"')

EXP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW_DIR = os.path.join(EXP_DIR, "data", "raw")


def git(*args):
    out = subprocess.run(["git", *args], cwd=EXP_DIR, check=True, capture_output=True)
    return out.stdout.decode("utf-8")


# 말뭉치


def corpus_paths():
    paths = git("ls-tree", "-r", "--full-tree", "--name-only", CORPUS_COMMIT).splitlines()
    picked = []
    for path in paths:
        if path.startswith("docs/archive/"):
            continue
        is_doc = path.endswith(".md") and (path.startswith("docs/") or "/" not in path)
        if is_doc or path.endswith(".rs"):
            picked.append(path)
    return sorted(picked)


def markdown_docs(path, text):
    docs, lines, start, in_fence = [], [], 1, False
    for number, line in enumerate(text.split("\n"), 1):
        if line.startswith("```"):
            in_fence = not in_fence
        if line.startswith("#") and not in_fence and lines:
            docs.append((start, number - 1, lines))
            lines, start = [], number
        lines.append(line)
    if lines:
        docs.append((start, start + len(lines) - 1, lines))
    out = []
    for index, (first, last, body) in enumerate(docs):
        body_text = "\n".join(line for line in body if line.strip())
        if len(body_text) < 20:
            continue
        out.append({
            "doc_id": f"{path}#s{index:03d}", "path": path, "kind": "markdown",
            "start_line": first, "end_line": last, "text": body_text,
            "ident_lines": [],
        })
    return out


def comment_start(line):
    in_string, escaped = False, False
    for i, ch in enumerate(line):
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
        elif ch == '"':
            in_string = True
        elif line.startswith("//", i):
            return i
    return len(line)


def code_line_parts(line):
    cut = comment_start(line)
    code, comment = line[:cut], line[cut + 2:]
    comment = comment.lstrip("/!").strip()
    code = STRING_LITERAL.sub(" ", code)
    idents = [w for w in IDENT.findall(code) if w not in RUST_KEYWORDS]
    return comment, idents


def code_docs(path, text):
    lines = text.split("\n")
    out = []
    for first in range(0, len(lines), CODE_CHUNK_LINES):
        chunk = lines[first:first + CODE_CHUNK_LINES]
        text_lines, ident_lines = [], []
        for line in chunk:
            comment, idents = code_line_parts(line)
            if idents:
                joined = " ".join(idents)
                text_lines.append(joined)
                ident_lines.append(joined)
            if comment:
                text_lines.append(comment)
        body_text = "\n".join(text_lines)
        if len(body_text) < 20:
            continue
        last = first + len(chunk)
        out.append({
            "doc_id": f"{path}#L{first + 1}-{last}", "path": path, "kind": "code",
            "start_line": first + 1, "end_line": last, "text": body_text,
            "ident_lines": ident_lines,
        })
    return out


def build_corpus():
    docs = []
    for path in corpus_paths():
        text = unicodedata.normalize("NFC", git("show", f"{CORPUS_COMMIT}:{path}"))
        docs.extend(markdown_docs(path, text) if path.endswith(".md") else code_docs(path, text))
    return docs


def collapse(text):
    return " ".join(text.split())


# 단어 조각


def char_kind(ch):
    code = ord(ch)
    if (0xAC00 <= code <= 0xD7A3 or 0x1100 <= code <= 0x11FF or 0x3130 <= code <= 0x318F
            or 0xA960 <= code <= 0xA97F or 0xD7B0 <= code <= 0xD7FF):
        return "hangul"
    if (0x4E00 <= code <= 0x9FFF or 0x3400 <= code <= 0x4DBF or 0xF900 <= code <= 0xFAFF
            or 0x3040 <= code <= 0x30FF):
        return "cjk"
    category = unicodedata.category(ch)
    if category == "Nd":
        return "digit"
    if category.startswith("L") and "LATIN" in unicodedata.name(ch, ""):
        return "latin"
    return None


def runs(text):
    out, kind, buf = [], None, []
    for ch in unicodedata.normalize("NFC", text):
        k = char_kind(ch)
        if k != kind and buf:
            if kind is not None:
                out.append((kind, "".join(buf)))
            buf = []
        kind = k
        buf.append(ch)
    if buf and kind is not None:
        out.append((kind, "".join(buf)))
    return out


def split_identifier(run):
    words, cur = [], run[0]
    for i in range(1, len(run)):
        prev, ch = run[i - 1], run[i]
        nxt = run[i + 1] if i + 1 < len(run) else ""
        if (prev.islower() and ch.isupper()) or (prev.isupper() and ch.isupper() and nxt.islower()):
            words.append(cur)
            cur = ch
        else:
            cur += ch
    words.append(cur)
    return [w.lower() for w in words]


def ngrams(seq, n):
    if len(seq) <= n:
        return [seq]
    return [seq[i:i + n] for i in range(len(seq) - n + 1)]


def tokenize(text, ko_units, en_units):
    out = []
    for kind, run in runs(text):
        if kind == "hangul":
            for unit in ko_units:
                if unit == "syl2":
                    out.extend("s:" + g for g in ngrams(run, 2))
                else:
                    n = int(unit[-1])
                    jamo = unicodedata.normalize("NFKD", run)
                    out.extend(f"j{n}:" + g for g in ngrams(jamo, n))
        elif kind == "cjk":
            out.extend("c:" + g for g in ngrams(run, 2))
        elif kind == "digit":
            out.append("d:" + run)
        else:
            for word in split_identifier(run):
                for unit in en_units:
                    if unit == "word":
                        out.append("w:" + word)
                    else:
                        out.extend("g:" + g for g in ngrams(word, 4))
    return out


# BM25


class Bm25:
    def __init__(self, doc_tokens):
        self.n_docs = len(doc_tokens)
        self.lengths = [len(t) for t in doc_tokens]
        self.avg_len = sum(self.lengths) / self.n_docs
        self.postings = defaultdict(list)
        for index, tokens in enumerate(doc_tokens):
            for term, tf in sorted(Counter(tokens).items()):
                self.postings[term].append((index, tf))

    def stats(self):
        return {
            "vocab_size": len(self.postings),
            "postings": sum(len(p) for p in self.postings.values()),
            "total_tokens": sum(self.lengths),
        }

    def rank(self, query_tokens):
        scores = defaultdict(float)
        for term in sorted(set(query_tokens)):
            posting = self.postings.get(term)
            if not posting:
                continue
            df = len(posting)
            idf = math.log(1 + (self.n_docs - df + 0.5) / (df + 0.5))
            for index, tf in posting:
                norm = 1 - BM25_B + BM25_B * self.lengths[index] / self.avg_len
                scores[index] += idf * tf * (BM25_K1 + 1) / (tf + BM25_K1 * norm)
        return sorted(((round(s, 9), i) for i, s in scores.items() if s > 0),
                      key=lambda item: (-item[0], item[1]))


# 질의


def gold_docs(text, collapsed):
    return [i for i, body in enumerate(collapsed) if text in body]


def sample_windows(positions, lines_of, sizes, valid, rng, collapsed):
    rng.shuffle(positions)
    chosen, seen_text, per_doc, used = [], set(), Counter(), set()
    for doc, line, start in positions:
        size = rng.choice(sizes)
        words = lines_of(doc)[line].split()
        window = words[start:start + size]
        if len(window) < size or not valid(window):
            continue
        span = {(doc, line, start + i) for i in range(size)}
        text = " ".join(window)
        if text in seen_text or per_doc[doc] >= MAX_QUERIES_PER_DOC or span & used:
            continue
        gold = gold_docs(text, collapsed)
        if len(gold) > MAX_GOLD_DOCS:
            continue
        chosen.append((doc, text, gold))
        seen_text.add(text)
        per_doc[doc] += 1
        used |= span
        if len(chosen) == QUERIES_PER_LANG:
            break
    return chosen


def ko_valid(window):
    return all(HANGUL_WORD.match(w) for w in window) and sum(len(w) for w in window) >= 4


def en_valid(window):
    if not all(IDENT_WORD.match(w) and len(w) >= 3 for w in window):
        return False
    return any(sum(c.isalpha() for c in w) >= 4 for w in window)


def ko_positions(docs):
    out = []
    for d, doc in enumerate(docs):
        for l, line in enumerate(doc["text"].split("\n")):
            words = line.split()
            for i in range(len(words) - 1):
                if HANGUL_WORD.match(words[i]) and HANGUL_WORD.match(words[i + 1]):
                    out.append((d, l, i))
    return out


def en_positions(docs):
    out = []
    for d, doc in enumerate(docs):
        for l, line in enumerate(doc["ident_lines"]):
            words = line.split()
            for i in range(len(words) - 1):
                out.append((d, l, i))
    return out


def ko_typo(text, kind, rng):
    if kind == "space-drop":
        spaces = [i for i, c in enumerate(text) if c == " "]
        i = rng.choice(spaces)
        return text[:i] + text[i + 1:], {"position": i}
    positions = [i for i, c in enumerate(text) if 0xAC00 <= ord(c) <= 0xD7A3]
    i = rng.choice(positions)
    value = ord(text[i]) - 0xAC00
    cho, jung, jong = value // 588, (value % 588) // 28, value % 28
    if kind == "jamo-sub":
        slot = rng.choice(["cho", "jung", "jong"] if jong else ["cho", "jung"])
        if slot == "cho":
            cho = rng.choice([x for x in range(19) if x != cho])
        elif slot == "jung":
            jung = rng.choice([x for x in range(21) if x != jung])
        else:
            jong = rng.choice([x for x in range(1, 28) if x != jong])
        new = chr(0xAC00 + cho * 588 + jung * 28 + jong)
        detail = {"position": i, "slot": slot}
    else:
        if jong:
            new = chr(0xAC00 + cho * 588 + jung * 28)
            detail = {"position": i, "slot": "jong"}
        else:
            new = COMPAT_CHOSEONG[cho]
            detail = {"position": i, "slot": "jung"}
    return text[:i] + new + text[i + 1:], detail


def en_typo(text, kind, rng):
    if kind == "space-drop":
        spaces = [i for i, c in enumerate(text) if c == " "]
        i = rng.choice(spaces)
        return text[:i] + text[i + 1:], {"position": i}
    words = text.split(" ")
    targets = [w for w, word in enumerate(words) if sum(c.isalpha() for c in word) >= 4]
    w = rng.choice(targets)
    word = words[w]
    letters = [i for i, c in enumerate(word) if c.isalpha()]
    alphabet = "abcdefghijklmnopqrstuvwxyz"
    if kind == "letter-sub":
        i = rng.choice(letters)
        new_char = rng.choice([c for c in alphabet if c != word[i].lower()])
        new_char = new_char.upper() if word[i].isupper() else new_char
        word = word[:i] + new_char + word[i + 1:]
    elif kind == "letter-ins":
        i = rng.randrange(len(word) + 1)
        word = word[:i] + rng.choice(alphabet) + word[i:]
    elif kind == "letter-del":
        i = rng.choice(letters)
        word = word[:i] + word[i + 1:]
    else:
        pairs = [i for i in range(len(word) - 1)
                 if word[i].isalpha() and word[i + 1].isalpha() and word[i] != word[i + 1]]
        i = rng.choice(pairs)
        word = word[:i] + word[i + 1] + word[i] + word[i + 2:]
    words[w] = word
    return " ".join(words), {"word": w, "position": i}


def build_queries(docs):
    rng = random.Random(SEED)
    collapsed = [collapse(doc["text"]) for doc in docs]
    ko = sample_windows(ko_positions(docs), lambda d: docs[d]["text"].split("\n"),
                        KO_WINDOW_WORDS, ko_valid, rng, collapsed)
    en = sample_windows(en_positions(docs), lambda d: docs[d]["ident_lines"],
                        EN_WINDOW_WORDS, en_valid, rng, collapsed)
    queries = []
    for lang, chosen, kinds, inject in (("ko", ko, KO_TYPOS, ko_typo), ("en", en, EN_TYPOS, en_typo)):
        for n, (doc, text, gold) in enumerate(chosen, 1):
            kind = rng.choice(kinds)
            typo_text, detail = inject(text, kind, rng)
            base = {
                "query_id": f"{lang}-{n:04d}", "lang": lang, "source_doc_id": docs[doc]["doc_id"],
                "clean_text": text, "gold_doc_ids": [docs[g]["doc_id"] for g in gold],
            }
            queries.append({**base, "variant": "clean", "typo_type": None,
                            "typo_detail": None, "query_text": text})
            queries.append({**base, "variant": "typo", "typo_type": kind,
                            "typo_detail": detail, "query_text": typo_text})
    return queries


# 실행


def compute(docs):
    queries = build_queries(docs)
    doc_ids = [doc["doc_id"] for doc in docs]
    index_rows, ranking_rows = [], []
    for condition, (ko_units, en_units) in CONDITIONS.items():
        index = Bm25([tokenize(doc["text"], ko_units, en_units) for doc in docs])
        index_rows.append({"trial_id": None, "condition": condition, **index.stats()})
        for q in queries:
            tokens = tokenize(q["query_text"], ko_units, en_units)
            ranked = index.rank(tokens)
            gold = set(q["gold_doc_ids"])
            gold_rank = next((r for r, (_, i) in enumerate(ranked, 1) if doc_ids[i] in gold), None)
            ranking_rows.append({
                "trial_id": f"{q['query_id']}-{q['variant']}", "condition": condition, **q,
                "n_query_tokens": len(set(tokens)), "n_retrieved": len(ranked),
                "gold_rank": gold_rank,
                "top_doc_ids": [doc_ids[i] for _, i in ranked[:TOP_K]],
                "top_scores": [round(s, 6) for s, _ in ranked[:TOP_K]],
            })
    return queries, index_rows, ranking_rows


def corpus_rows(docs):
    return [{"trial_id": None, "condition": None,
             **{k: v for k, v in doc.items() if k != "ident_lines"}} for doc in docs]


def write_jsonl(path, rows, run_id, ts):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps({"run_id": run_id, "ts_utc": ts, **row}, ensure_ascii=False) + "\n")


def read_jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f]


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def sysctl(name):
    try:
        return subprocess.run(["sysctl", "-n", name], check=True, capture_output=True,
                              text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def write_env(run_id, started, finished):
    memory = sysctl("hw.memsize")
    env = {
        "os": platform.platform(),
        "cpu": sysctl("machdep.cpu.brand_string") or platform.processor(),
        "memory_bytes": int(memory) if memory else None,
        "tools": {"python": platform.python_version(), "git": git("--version").strip()},
        "model": None,
        "run_id": run_id,
        "run_started_utc": started,
        "run_finished_utc": finished,
        "repo_commit": git("rev-parse", "HEAD").strip(),
        "corpus_commit": CORPUS_COMMIT,
        "seed": SEED,
        "parameters": {
            "queries_per_lang": QUERIES_PER_LANG, "max_queries_per_doc": MAX_QUERIES_PER_DOC,
            "max_gold_docs": MAX_GOLD_DOCS, "code_chunk_lines": CODE_CHUNK_LINES,
            "top_k": TOP_K, "bm25_k1": BM25_K1, "bm25_b": BM25_B,
        },
    }
    with open(os.path.join(EXP_DIR, "env.json"), "w", encoding="utf-8") as f:
        json.dump(env, f, ensure_ascii=False, indent=2)
        f.write("\n")


def collect():
    started = datetime.datetime.now(datetime.timezone.utc)
    run_id = started.strftime("%Y%m%dT%H%M%SZ") + "-" + git("rev-parse", "--short=7", "HEAD").strip()
    os.makedirs(RAW_DIR, exist_ok=True)
    docs = build_corpus()
    _, index_rows, ranking_rows = compute(docs)
    ts = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    names = {"corpus": corpus_rows(docs), "index": index_rows, "rankings": ranking_rows}
    for name, rows in names.items():
        write_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"), rows, run_id, ts)
    write_env(run_id, started.strftime("%Y-%m-%dT%H:%M:%SZ"), ts)
    raw_files = sorted(f for f in os.listdir(RAW_DIR) if f.endswith(".jsonl"))
    with open(os.path.join(EXP_DIR, "data", "SHA256SUMS"), "w", encoding="utf-8") as f:
        for name in raw_files:
            f.write(f"{sha256(os.path.join(RAW_DIR, name))}  raw/{name}\n")
    print(f"run_id {run_id}: docs {len(docs)}, ranking rows {len(ranking_rows)}")


def latest_run_id():
    ids = sorted(f[len("rankings-"):-len(".jsonl")] for f in os.listdir(RAW_DIR)
                 if f.startswith("rankings-"))
    if not ids:
        sys.exit("data/raw/에 rankings 파일이 없다")
    return ids[-1]


def strip_meta(rows):
    return [{k: v for k, v in row.items() if k not in ("run_id", "ts_utc")} for row in rows]


def verify():
    run_id = latest_run_id()
    docs = build_corpus()
    _, index_rows, ranking_rows = compute(docs)
    expected = {"corpus": corpus_rows(docs), "index": index_rows, "rankings": ranking_rows}
    failed = False
    for name, rows in expected.items():
        stored = read_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"))
        same = strip_meta(stored) == json.loads(json.dumps(rows, ensure_ascii=False))
        print(f"{name}-{run_id}.jsonl: {'일치' if same else '불일치'} ({len(stored)}행)")
        failed |= not same
    if failed:
        sys.exit(1)


def plan():
    docs = build_corpus()
    kinds = Counter(doc["kind"] for doc in docs)
    print(f"docs {len(docs)} {dict(kinds)}")
    print(f"ko positions {len(ko_positions(docs))}, en positions {len(en_positions(docs))}")
    queries = build_queries(docs)
    print(f"queries {Counter((q['lang'], q['variant']) for q in queries)}")


if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    actions = {"collect": collect, "verify": verify, "plan": plan}
    if command not in actions:
        sys.exit("사용법: 01-collect.py collect|verify|plan")
    actions[command]()
