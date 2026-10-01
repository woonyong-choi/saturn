"""Saturn 저장소 고정 커밋에서 한국어 설명과 영문 식별자 짝, 순위 질의를 만들고 임베딩 결과와 비용을 기록한다.

명령:
    collect  말뭉치, 질의, 두 모델의 짝 제안과 순위, 라벨 시트를 data/raw/에 새 실행 id로 쓴다.
    cost     모델마다 새 프로세스에서 설치 크기, 상주 메모리, 문장당 지연을 재서 data/raw/에 쓴다.
    seal     라벨 파일을 확인하고 env.json과 SHA256SUMS를 갱신한다.
    verify   가장 최근 실행의 결정적 raw 파일을 같은 입력으로 다시 계산한 값과 비교한다.
    plan     말뭉치 문서 수와 설명 후보 수만 출력한다. 임베딩은 계산하지 않는다.
"""

import csv
import datetime
import hashlib
import importlib.metadata
import json
import math
import os
import platform
import random
import re
import resource
import subprocess
import sys
import time
import unicodedata
from collections import Counter, defaultdict

CORPUS_COMMIT = "1258e17dd1a6d527249b0aaa9dccd96aef8aec2a"
SEED = 155
N_PAIR_QUERIES = 200
N_GUARD_PER_LANG = 200
MAX_QUERIES_PER_DOC = 8
MAX_GOLD_DOCS = 5
CODE_CHUNK_LINES = 40
TOP_K = 10
RRF_K = 60
BM25_K1 = 1.2
BM25_B = 0.75
MIN_HANGUL = 6
PROPOSAL_TOP = 5
COST_WARMUP = 20
COST_ROUNDS = 3

MODELS = {
    "e5": {
        "name": "intfloat/multilingual-e5-small",
        "hf": "intfloat/multilingual-e5-small",
        "revision": "614241f622f53c4eeff9890bdc4f31cfecc418b3",
        "model_file": "onnx/model.onnx",
        "model_bytes": 470268510,
        "model_sha256": "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
        "pair_prefix": ("query: ", "query: "),
        "rank_prefix": ("query: ", "passage: "),
        "custom": True,
    },
    "minilm": {
        "name": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        "hf": "qdrant/paraphrase-multilingual-MiniLM-L12-v2-onnx-Q",
        "revision": "faf4aa4225822f3bc6376869cb1164e8e3feedd0",
        "model_file": "model_optimized.onnx",
        "model_bytes": 235052644,
        "model_sha256": "634d0f66c29dc934c8fa72b8a4fe91dd4d420a22f1d82a241058d4316e659a99",
        "pair_prefix": ("", ""),
        "rank_prefix": ("", ""),
        "custom": False,
    },
}

RUST_KEYWORDS = frozenset(
    "as async await break const continue crate dyn else enum extern false fn for if impl in "
    "let loop match mod move mut pub ref return self Self static struct super trait true type "
    "unsafe use where while".split()
)
HANGUL_WORD = re.compile(r"^[가-힣]+$")
IDENT_WORD = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
STRING_LITERAL = re.compile(r'r#*"[^"]*"#*|"(?:\\.|[^"\\])*"')
DOC_COMMENT = re.compile(r"^\s*///(?!/)\s?(.*)$")
ATTRIBUTE = re.compile(r"^\s*#\[")
VIS = r"(?:pub(?:\([^)]*\))?\s+)?"
DEF_ITEM = re.compile(
    r"^\s*" + VIS + r"(?:(?:async|const|unsafe|extern\s+\"C\")\s+)*"
    r"(fn|struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
DEF_FIELD = re.compile(r"^\s*" + VIS + r"([a-z_][A-Za-z0-9_]*)\s*:(?!:)")
DEF_VARIANT = re.compile(r"^\s*([A-Z][A-Za-z0-9_]*)\s*(?:[,({=]|$)")
CODE_SPAN = re.compile(r"`[^`]*`")
LATIN_TOKEN = re.compile(r"[A-Za-z0-9_:./<>\-\[\]()#*&^%$@!?+=|\\]+")
SENTENCE_END = re.compile(r"(?<=[다요음함됨임])\.(?:\s|$)|\.\s")

EXP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW_DIR = os.path.join(EXP_DIR, "data", "raw")


def git(*args):
    out = subprocess.run(["git", *args], cwd=EXP_DIR, check=True, capture_output=True)
    return out.stdout.decode("utf-8")


def show(path):
    return unicodedata.normalize("NFC", git("show", f"{CORPUS_COMMIT}:{path}"))


def tracked_paths():
    return sorted(git("ls-tree", "-r", "--full-tree", "--name-only", CORPUS_COMMIT).splitlines())


# 짝: Rust 문서 주석과 바로 아래 정의


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


def identifier_words(name):
    words = []
    for part in re.split(r"[_\W]+", name):
        if part:
            words.extend(split_identifier(part))
    return " ".join(words)


def definition_name(line):
    m = DEF_ITEM.match(line)
    if m:
        return m.group(1), m.group(2)
    m = DEF_FIELD.match(line)
    if m and m.group(1) not in RUST_KEYWORDS:
        return "field", m.group(1)
    m = DEF_VARIANT.match(line)
    if m and m.group(1) not in RUST_KEYWORDS:
        return "variant", m.group(1)
    return None


def korean_description(block):
    text = " ".join(line.strip() for line in block).strip()
    m = SENTENCE_END.search(text)
    first = text[:m.start() + 1] if m else text
    first = CODE_SPAN.sub(" ", first)
    first = LATIN_TOKEN.sub(" ", first)
    first = re.sub(r"[^\w\s가-힣]", " ", first)
    first = re.sub(r"[A-Za-z0-9_]", " ", first)
    first = " ".join(first.split())
    if sum(1 for ch in first if "가" <= ch <= "힣") < MIN_HANGUL:
        return None
    return first


def rust_paths():
    return [p for p in tracked_paths() if p.endswith(".rs")]


def extract_definitions():
    """정의 전체(식별자 후보 집합)와 한국어 문서 주석이 붙은 정의(설명 짝 후보)를 만든다."""
    pool, described = set(), []
    for path in rust_paths():
        lines = show(path).split("\n")
        block, block_start, attr_depth = [], None, 0
        for number, line in enumerate(lines, 1):
            m = DOC_COMMENT.match(line)
            if m:
                if not block:
                    block_start = number
                block.append(m.group(1))
                continue
            if block and (attr_depth > 0 or ATTRIBUTE.match(line)):
                attr_depth += line.count("[") - line.count("]")
                continue
            item = DEF_ITEM.match(line)
            if item:
                pool.add(item.group(2))
            if block:
                found = definition_name(line)
                if found:
                    kind, name = found
                    pool.add(name)
                    desc = korean_description(block)
                    if desc:
                        described.append({
                            "def_id": f"{path}#L{number}", "path": path, "line": number,
                            "comment_line": block_start, "kind": kind, "identifier": name,
                            "description": desc,
                        })
            block = []
    return sorted(pool), described


def sample_pair_queries(described):
    rng = random.Random(SEED)
    seen, unique = set(), []
    for row in described:
        key = row["description"]
        if key in seen:
            continue
        seen.add(key)
        unique.append(row)
    rng.shuffle(unique)
    chosen = unique[:N_PAIR_QUERIES]
    order = list(range(len(chosen)))
    rng.shuffle(order)
    out = []
    for n, row in enumerate(chosen, 1):
        out.append({"query_id": f"pair-{n:03d}", "fold": order[n - 1] % 2, **row})
    return out


# 순위 말뭉치


def corpus_paths():
    picked = []
    for path in tracked_paths():
        if path.startswith("docs/archive/") or path.startswith("docs/experiments/"):
            continue
        is_doc = path.endswith(".md") and (path.startswith("docs/") or "/" not in path)
        if is_doc or path.endswith(".rs"):
            picked.append(path)
    return picked


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
        out.append({"doc_id": f"{path}#s{index:03d}", "path": path, "kind": "markdown",
                    "start_line": first, "end_line": last, "text": body_text,
                    "embed_text": body_text})
    return out


def code_identifiers(line):
    cut = line.find("//")
    code = line if cut < 0 else line[:cut]
    code = STRING_LITERAL.sub(" ", code)
    return [w for w in IDENT.findall(code) if w not in RUST_KEYWORDS]


def code_docs(path, text):
    lines = text.split("\n")
    out = []
    for first in range(0, len(lines), CODE_CHUNK_LINES):
        chunk = lines[first:first + CODE_CHUNK_LINES]
        ident_lines = [" ".join(ids) for ids in (code_identifiers(l) for l in chunk) if ids]
        body_text = "\n".join(ident_lines)
        if len(body_text) < 20:
            continue
        last = first + len(chunk)
        embed = "\n".join(" ".join(identifier_words(w) for w in l.split()) for l in ident_lines)
        out.append({"doc_id": f"{path}#L{first + 1}-{last}", "path": path, "kind": "code",
                    "start_line": first + 1, "end_line": last, "text": body_text,
                    "embed_text": embed})
    return out


def build_corpus():
    docs = []
    for path in corpus_paths():
        text = show(path)
        docs.extend(markdown_docs(path, text) if path.endswith(".md") else code_docs(path, text))
    return docs


def collapse(text):
    return " ".join(text.split())


# 단어 조각과 BM25: 맥락 고르기의 기준 규칙(한글 글자 2개, 영문 식별자 단어)


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


def ngrams(seq, n):
    if len(seq) <= n:
        return [seq]
    return [seq[i:i + n] for i in range(len(seq) - n + 1)]


def tokenize(text):
    out = []
    for kind, run in runs(text):
        if kind in ("hangul", "cjk"):
            out.extend("s:" + g for g in ngrams(run, 2))
        elif kind == "digit":
            out.append("d:" + run)
        else:
            out.extend("w:" + w for w in split_identifier(run))
    return out


class Bm25:
    def __init__(self, doc_tokens):
        self.n_docs = len(doc_tokens)
        self.lengths = [len(t) for t in doc_tokens]
        self.avg_len = sum(self.lengths) / self.n_docs
        self.postings = defaultdict(list)
        for index, tokens in enumerate(doc_tokens):
            for term, tf in sorted(Counter(tokens).items()):
                self.postings[term].append((index, tf))

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
        return [i for _, i in sorted(((round(s, 9), i) for i, s in scores.items() if s > 0),
                                     key=lambda item: (-item[0], item[1]))]


def rrf(lists):
    scores = defaultdict(float)
    for ranked in lists:
        for r, i in enumerate(ranked, 1):
            scores[i] += 1 / (RRF_K + r)
    return [i for _, i in sorted(((round(s, 12), i) for i, s in scores.items()),
                                 key=lambda item: (-item[0], item[1]))]


# 순위 질의


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
        if not gold or len(gold) > MAX_GOLD_DOCS:
            continue
        chosen.append((doc, text, gold))
        seen_text.add(text)
        per_doc[doc] += 1
        used |= span
        if len(chosen) == N_GUARD_PER_LANG:
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
        if doc["kind"] != "markdown":
            continue
        for n, line in enumerate(doc["text"].split("\n")):
            words = line.split()
            for i in range(len(words) - 1):
                if HANGUL_WORD.match(words[i]) and HANGUL_WORD.match(words[i + 1]):
                    out.append((d, n, i))
    return out


def en_positions(docs):
    out = []
    for d, doc in enumerate(docs):
        if doc["kind"] != "code":
            continue
        for n, line in enumerate(doc["text"].split("\n")):
            words = line.split()
            for i in range(len(words) - 1):
                out.append((d, n, i))
    return out


def chunk_of(docs, path, line):
    for i, doc in enumerate(docs):
        if doc["kind"] == "code" and doc["path"] == path and doc["start_line"] <= line <= doc["end_line"]:
            return i
    return None


def build_rank_queries(docs, pair_queries):
    rng = random.Random(SEED)
    collapsed = [collapse(doc["text"]) for doc in docs]
    queries = []
    for q in pair_queries:
        gold = chunk_of(docs, q["path"], q["line"])
        queries.append({"rank_query_id": f"syn-{q['query_id']}", "set": "synonym",
                        "pair_query_id": q["query_id"], "query_text": q["description"],
                        "gold_doc_ids": [docs[gold]["doc_id"]] if gold is not None else []})
    ko = sample_windows(ko_positions(docs), lambda d: docs[d]["text"].split("\n"),
                        (2, 3, 4), ko_valid, rng, collapsed)
    en = sample_windows(en_positions(docs), lambda d: docs[d]["text"].split("\n"),
                        (2, 3), en_valid, rng, collapsed)
    for lang, chosen in (("ko", ko), ("en", en)):
        for n, (_, text, gold) in enumerate(chosen, 1):
            queries.append({"rank_query_id": f"lex-{lang}-{n:03d}", "set": f"lexical-{lang}",
                            "pair_query_id": None, "query_text": text,
                            "gold_doc_ids": [docs[g]["doc_id"] for g in gold]})
    return queries


# 임베딩


def load_model(key):
    from fastembed import TextEmbedding
    spec = MODELS[key]
    if spec["custom"]:
        from fastembed.common.model_description import ModelSource, PoolingType
        known = {m["model"] for m in TextEmbedding.list_supported_models()}
        if spec["name"] not in known:
            TextEmbedding.add_custom_model(
                model=spec["name"], pooling=PoolingType.MEAN, normalization=True,
                sources=ModelSource(hf=spec["hf"]), dim=384, model_file=spec["model_file"])
    return TextEmbedding(spec["name"])


def model_snapshot(key):
    from huggingface_hub import snapshot_download
    spec = MODELS[key]
    from fastembed.common.utils import define_cache_dir
    return snapshot_download(spec["hf"], revision=spec["revision"], cache_dir=str(define_cache_dir()),
                             local_files_only=True)


def check_model_file(key):
    spec = MODELS[key]
    path = os.path.join(model_snapshot(key), spec["model_file"])
    size = os.path.getsize(path)
    digest = sha256(path)
    if size != spec["model_bytes"] or digest != spec["model_sha256"]:
        sys.exit(f"{spec['name']} 모델 파일이 고정한 크기, 해시와 다르다: {size} {digest}")
    return path


def embed(model, texts, batch_size=32):
    import numpy as np
    out = []
    for v in model.embed(texts, batch_size=batch_size):
        v = np.asarray(v, dtype=np.float64)
        n = float(np.linalg.norm(v))
        out.append(v / n if n else v)
    return np.vstack(out)


def compute_embeddings(pool, pair_queries, docs, rank_queries):
    import numpy as np
    pool_words = [identifier_words(name) for name in pool]
    proposals, rankings = [], []
    bm25 = Bm25([tokenize(doc["text"]) for doc in docs])
    base_lists = [bm25.rank(tokenize(q["query_text"])) for q in rank_queries]
    doc_ids = [doc["doc_id"] for doc in docs]
    for key, spec in MODELS.items():
        check_model_file(key)
        model = load_model(key)
        qp, ip = spec["pair_prefix"]
        q_vecs = embed(model, [qp + q["description"] for q in pair_queries])
        i_vecs = embed(model, [ip + w for w in pool_words])
        sims = q_vecs @ i_vecs.T
        for qi, q in enumerate(pair_queries):
            order = sorted(range(len(pool)), key=lambda j: (-round(float(sims[qi, j]), 6), pool[j]))
            top = order[:PROPOSAL_TOP]
            gold_j = pool.index(q["identifier"])
            gold_words = pool_words[gold_j]
            gold_rank = next(r for r, j in enumerate(order, 1) if pool_words[j] == gold_words)
            proposals.append({
                "trial_id": f"{q['query_id']}-{key}", "condition": key, "query_id": q["query_id"],
                "top_identifiers": [pool[j] for j in top],
                "top_scores": [round(float(sims[qi, j]), 6) for j in top],
                "gold_identifier": q["identifier"],
                "gold_score": round(float(sims[qi, gold_j]), 6), "gold_rank": gold_rank,
            })
        rq, rd = spec["rank_prefix"]
        d_vecs = embed(model, [rd + doc["embed_text"] for doc in docs], batch_size=16)
        r_vecs = embed(model, [rq + q["query_text"] for q in rank_queries])
        dsims = r_vecs @ d_vecs.T
        for qi, q in enumerate(rank_queries):
            emb_list = sorted(range(len(docs)), key=lambda j: (-round(float(dsims[qi, j]), 6), j))
            fused = rrf([base_lists[qi], emb_list])
            gold = set(q["gold_doc_ids"])
            row = {"trial_id": f"{q['rank_query_id']}-{key}", "condition": key,
                   "rank_query_id": q["rank_query_id"], "set": q["set"]}
            for name, ranked in (("base", base_lists[qi]), ("embedding", emb_list), ("rrf", fused)):
                row[f"{name}_gold_rank"] = next(
                    (r for r, i in enumerate(ranked, 1) if doc_ids[i] in gold), None)
                row[f"{name}_top_doc_ids"] = [doc_ids[i] for i in ranked[:TOP_K]]
            rankings.append(row)
        del model
    return proposals, rankings


def label_sheet(pair_queries, proposals):
    by_query = {q["query_id"]: q for q in pair_queries}
    keys = set()
    for q in pair_queries:
        keys.add((q["query_id"], q["identifier"]))
    for p in proposals:
        keys.add((p["query_id"], p["top_identifiers"][0]))
    rows = sorted(keys)
    random.Random(SEED).shuffle(rows)
    return [{"trial_id": None, "condition": None, "row_id": f"r{n:04d}", "query_id": qid,
             "korean": by_query[qid]["description"], "identifier": ident}
            for n, (qid, ident) in enumerate(rows, 1)]


# 비용


def dist_bytes(name):
    dist = importlib.metadata.distribution(name)
    total = 0
    for f in dist.files or []:
        path = dist.locate_file(f)
        if os.path.isfile(path):
            total += os.path.getsize(path)
    return total


def snapshot_bytes(key):
    root = model_snapshot(key)
    total = 0
    for base, _, files in os.walk(root):
        for f in files:
            total += os.path.getsize(os.path.realpath(os.path.join(base, f)))
    return total


def rss_bytes():
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(os.getpid())], check=True,
                         capture_output=True, text=True).stdout
    return int(out.strip()) * 1024


def cost_child(key, sentences_path):
    """새 프로세스 안에서 한 모델의 메모리와 지연을 잰다."""
    import fastembed  # noqa: F401
    import numpy  # noqa: F401
    rss_before = rss_bytes()
    check_model_file(key)
    t0 = time.perf_counter_ns()
    model = load_model(key)
    load_ns = time.perf_counter_ns() - t0
    with open(sentences_path, encoding="utf-8") as f:
        sentences = [json.loads(line)["text"] for line in f]
    prefix = MODELS[key]["pair_prefix"][0]
    for s in sentences[:COST_WARMUP]:
        list(model.embed([prefix + s], batch_size=1))
    latencies = []
    for round_index in range(COST_ROUNDS):
        for i, s in enumerate(sentences):
            t = time.perf_counter_ns()
            list(model.embed([prefix + s], batch_size=1))
            latencies.append({"round": round_index, "sentence": i,
                              "ns": time.perf_counter_ns() - t})
    rss_after = rss_bytes()
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    print(json.dumps({"rss_before": rss_before, "rss_after": rss_after, "peak_rss": peak,
                      "load_ns": load_ns, "latencies": latencies}))


def cost():
    run_id = latest_run_id()
    queries = read_jsonl(os.path.join(RAW_DIR, f"pairs-{run_id}.jsonl"))
    sentences_path = os.path.join(RAW_DIR, f"cost-input-{run_id}.jsonl")
    with open(sentences_path, "w", encoding="utf-8", newline="\n") as f:
        for q in queries:
            f.write(json.dumps({"text": q["description"]}, ensure_ascii=False) + "\n")
    rows = []
    ts = now()
    runtime = {name: dist_bytes(name) for name in ("onnxruntime", "tokenizers")}
    for key in MODELS:
        out = subprocess.run([sys.executable, __file__, "cost-child", key, sentences_path],
                             check=True, capture_output=True, text=True).stdout
        result = json.loads(out.strip().splitlines()[-1])
        snap = snapshot_bytes(key)
        base = {"run_id": run_id, "ts_utc": ts, "condition": key}
        rows.append({**base, "trial_id": f"size-{key}", "measure": "install_bytes",
                     "value": snap + sum(runtime.values()),
                     "detail": {"model_snapshot": snap, **runtime}})
        rows.append({**base, "trial_id": f"memory-{key}", "measure": "rss_increase_bytes",
                     "value": result["rss_after"] - result["rss_before"],
                     "detail": {k: result[k] for k in ("rss_before", "rss_after", "peak_rss", "load_ns")}})
        for lat in result["latencies"]:
            rows.append({**base, "trial_id": f"latency-{key}-{lat['round']}-{lat['sentence']:03d}",
                         "measure": "latency_ns", "value": lat["ns"],
                         "detail": {"round": lat["round"], "sentence": lat["sentence"]}})
    with open(os.path.join(RAW_DIR, f"cost-{run_id}.jsonl"), "w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(f"cost {run_id}: {len(rows)} rows")


# 실행


def now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def write_jsonl(path, rows, run_id, ts):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps({"run_id": run_id, "ts_utc": ts, **row}, ensure_ascii=False) + "\n")


def read_jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sysctl(name):
    try:
        return subprocess.run(["sysctl", "-n", name], check=True, capture_output=True,
                              text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def compute_deterministic():
    pool, described = extract_definitions()
    pair_queries = sample_pair_queries(described)
    docs = build_corpus()
    rank_queries = build_rank_queries(docs, pair_queries)
    return pool, described, pair_queries, docs, rank_queries


def collect():
    started = now()
    run_id = (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-"
              + git("rev-parse", "--short=7", "HEAD").strip())
    os.makedirs(RAW_DIR, exist_ok=True)
    pool, _, pair_queries, docs, rank_queries = compute_deterministic()
    proposals, rankings = compute_embeddings(pool, pair_queries, docs, rank_queries)
    ts = now()
    meta = lambda rows: [{"trial_id": None, "condition": None, **r} for r in rows]  # noqa: E731
    files = {
        "identifiers": meta([{"identifier": name, "words": identifier_words(name)} for name in pool]),
        "pairs": meta(pair_queries),
        "corpus": meta([{k: v for k, v in d.items()} for d in docs]),
        "rank-queries": meta(rank_queries),
        "proposals": proposals,
        "rankings": rankings,
        "label-sheet": label_sheet(pair_queries, proposals),
    }
    for name, rows in files.items():
        write_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"), rows, run_id, ts)
    write_env(run_id, started, None)
    print(f"run_id {run_id}: identifiers {len(pool)}, pairs {len(pair_queries)}, docs {len(docs)}, "
          f"rank queries {len(rank_queries)}, label rows {len(files['label-sheet'])}")


def latest_run_id():
    ids = sorted(f[len("pairs-"):-len(".jsonl")] for f in os.listdir(RAW_DIR)
                 if f.startswith("pairs-"))
    if not ids:
        sys.exit("data/raw/에 pairs 파일이 없다")
    return ids[-1]


def seal():
    run_id = latest_run_id()
    sheet = read_jsonl(os.path.join(RAW_DIR, f"label-sheet-{run_id}.jsonl"))
    labels_path = os.path.join(RAW_DIR, f"labels-{run_id}.csv")
    with open(labels_path, encoding="utf-8", newline="") as f:
        labels = {row["row_id"]: row["label"] for row in csv.DictReader(f)}
    missing = [r["row_id"] for r in sheet if labels.get(r["row_id"]) not in ("same", "different", "unclear")]
    if missing:
        sys.exit(f"라벨이 없거나 값이 틀린 행 {len(missing)}개: {missing[:5]}")
    with open(os.path.join(EXP_DIR, "env.json"), encoding="utf-8") as f:
        started = json.load(f)["run_started_utc"]
    write_env(run_id, started, now())
    raw_files = sorted(f for f in os.listdir(RAW_DIR) if f.endswith((".jsonl", ".csv")))
    with open(os.path.join(EXP_DIR, "data", "SHA256SUMS"), "w", encoding="utf-8") as f:
        for name in raw_files:
            f.write(f"{sha256(os.path.join(RAW_DIR, name))}  raw/{name}\n")
    print(f"sealed {run_id}: {len(raw_files)} raw files")


def write_env(run_id, started, finished):
    memory = sysctl("hw.memsize")
    versions = {}
    for name in ("fastembed", "onnxruntime", "tokenizers", "huggingface-hub", "numpy"):
        try:
            versions[name] = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            versions[name] = None
    env = {
        "os": platform.platform(),
        "cpu": sysctl("machdep.cpu.brand_string") or platform.processor(),
        "cpu_cores": os.cpu_count(),
        "memory_bytes": int(memory) if memory else None,
        "tools": {"python": platform.python_version(), "git": git("--version").strip(), **versions},
        "models": {k: {f: v for f, v in s.items() if f not in ("custom",)} for k, s in MODELS.items()},
        "run_id": run_id,
        "run_started_utc": started,
        "run_finished_utc": finished,
        "repo_commit": git("rev-parse", "HEAD").strip(),
        "corpus_commit": CORPUS_COMMIT,
        "seed": SEED,
        "parameters": {
            "pair_queries": N_PAIR_QUERIES, "guard_queries_per_lang": N_GUARD_PER_LANG,
            "max_queries_per_doc": MAX_QUERIES_PER_DOC, "max_gold_docs": MAX_GOLD_DOCS,
            "code_chunk_lines": CODE_CHUNK_LINES, "top_k": TOP_K, "rrf_k": RRF_K,
            "bm25_k1": BM25_K1, "bm25_b": BM25_B, "min_hangul": MIN_HANGUL,
            "cost_warmup": COST_WARMUP, "cost_rounds": COST_ROUNDS,
        },
    }
    with open(os.path.join(EXP_DIR, "env.json"), "w", encoding="utf-8") as f:
        json.dump(env, f, ensure_ascii=False, indent=2)
        f.write("\n")


def strip_meta(rows, drop=("run_id", "ts_utc")):
    return [{k: v for k, v in row.items() if k not in drop} for row in rows]


def verify():
    run_id = latest_run_id()
    pool, _, pair_queries, docs, rank_queries = compute_deterministic()
    meta = lambda rows: [{"trial_id": None, "condition": None, **r} for r in rows]  # noqa: E731
    expected = {
        "identifiers": meta([{"identifier": n, "words": identifier_words(n)} for n in pool]),
        "pairs": meta(pair_queries),
        "corpus": meta(docs),
        "rank-queries": meta(rank_queries),
    }
    failed = False
    for name, rows in expected.items():
        stored = read_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"))
        same = strip_meta(stored) == json.loads(json.dumps(rows, ensure_ascii=False))
        print(f"{name}-{run_id}.jsonl: {'일치' if same else '불일치'} ({len(stored)}행)")
        failed |= not same
    proposals, rankings = compute_embeddings(pool, pair_queries, docs, rank_queries)
    for name, rows, keys in (("proposals", proposals, ("top_identifiers", "gold_rank")),
                             ("rankings", rankings, ("base_gold_rank", "embedding_gold_rank", "rrf_gold_rank"))):
        stored = {r["trial_id"]: r for r in read_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"))}
        diff = sum(1 for r in rows if any(stored[r["trial_id"]][k] != r[k] for k in keys))
        print(f"{name}-{run_id}.jsonl: 다른 행 {diff}개 ({len(rows)}행, 비교 열 {', '.join(keys)})")
        failed |= diff > 0
    if failed:
        sys.exit(1)


def plan():
    pool, described, pair_queries, docs, rank_queries = compute_deterministic()
    print(f"identifiers {len(pool)}, described definitions {len(described)}, "
          f"unique descriptions {len({d['description'] for d in described})}")
    print(f"docs {len(docs)} {dict(Counter(d['kind'] for d in docs))}")
    print(f"rank queries {dict(Counter(q['set'] for q in rank_queries))}, "
          f"synonym without gold {sum(1 for q in rank_queries if not q['gold_doc_ids'])}")


if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "cost-child":
        cost_child(sys.argv[2], sys.argv[3])
        sys.exit(0)
    actions = {"collect": collect, "cost": cost, "seal": seal, "verify": verify, "plan": plan}
    if command not in actions:
        sys.exit("사용법: 01-collect.py collect|cost|seal|verify|plan")
    actions[command]()
