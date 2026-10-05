"""검색 선별 추가 이득 측정의 표본 생성기. 계열 하나는 한 프로젝트의 한 질문과 후보 N개(실제 길이)다.

층: exact(쉬운 정확 일치), none(후보에 근거 없음), other(같은 속성의 다른 프로젝트), stale(구버전과 정정),
synonym(질문과 근거가 다른 단어), combine(두 근거 결합), middle(긴 본문 중간 근거).
정답과 근거 위치(support_ids)는 후보와 분리해 두고 선택·요청에는 넣지 않는다.
"""

from __future__ import annotations

import hashlib
import json
import random

SEED = 541001
STRATA = ("exact", "none", "other", "stale", "synonym", "combine", "middle")
LANGS = ("en", "ko")
LENGTHS = {"short": 24, "long": 96}
SEALED_PER_CELL = 9  # 7 x 2 x 2 x 9 = 252 계열
DEV_PER_CELL = 1  # 7 x 2 x 2 x 1 = 28 계열
BUDGET_BYTES = 9000

ATTRS = [
    # (영문 속성, 한글 속성, 값 종류, 동의어 질문 영문, 동의어 질문 한글, 근거 영문, 근거 한글)
    ("port", "포트", "int", "which number does {p} accept connections on", "{p}가 접속을 받는 번호가 무엇인가", "{p} binds to :{v}", "{p}는 {v}번에 바인딩한다"),
    ("region", "리전", "word", "where is {p} hosted", "{p}가 올라가 있는 곳이 어디인가", "{p} runs in {v}", "{p}는 {v}에서 돈다"),
    ("retention", "보존 일수", "int", "how long are {p} records kept in days", "{p} 기록을 며칠 두는가", "{p} keeps data for {v} days", "{p}는 {v}일 동안 데이터를 둔다"),
    ("timeout", "제한 시간", "int", "how many seconds before {p} gives up", "{p}는 몇 초 뒤에 포기하는가", "{p} gives up after {v} seconds", "{p}는 {v}초 뒤에 포기한다"),
]
OTHER_ATTRS = ["owner", "language", "cluster", "tier"]
WORDS = ["seoul", "osaka", "oslo", "lima", "pune", "turin", "cairo", "quito", "perth", "bern"]
PEOPLE = ["mina", "joon", "ravi", "elena", "tomas", "hana", "omar", "lucia"]
HOSTS = ["db1", "db2", "cache3", "queue4", "edge5", "gw6"]
NOISE_EN = [
    "{t} INFO worker-{n} flushed {m} rows to queue {q}",
    "{t} WARN retry {n} of 5 for upstream {q} after {m} ms",
    "{t} DEBUG cache miss key={q}:{m} shard={n}",
    "diff --git a/{q}/mod{n}.rs b/{q}/mod{n}.rs index {m}..{n}",
    "test result: ok. {m} passed; 0 failed; {n} ignored; finished in {n}.{m}s",
    "$ grep -rn handler_{n} src/{q} | head -{m}",
    "TODO({q}): revisit batching strategy before release {n}.{m}",
    "build step {n}/{m} compile {q} done",
]
NOISE_KO = [
    "{t} INFO 작업자 {n}가 큐 {q}에 {m}행을 반영했다",
    "{t} WARN 상위 {q} 재시도 {n}회째, 지연 {m}ms",
    "{t} DEBUG 캐시 미스 키={q}:{m} 샤드={n}",
    "{q}/mod{n}.rs 수정, 줄 {m}개 바뀜",
    "테스트 {m}개 통과, 실패 0, 건너뜀 {n}, 소요 {n}.{m}초",
    "메모: {q} 배치 방식은 릴리스 {n}.{m} 전에 다시 본다",
    "빌드 단계 {n}/{m} {q} 완료",
]
PROJECT_STEMS = ["billing", "search", "auth", "ingest", "export", "notify", "report", "sync", "audit", "media"]
PROJECT_SUFFIX = ["api", "worker", "gateway", "batch", "cron", "proxy"]


def _line(rng: random.Random, lang: str, names: list[str]) -> str:
    tpl = rng.choice(NOISE_KO if lang == "ko" else NOISE_EN)
    t = f"2026-09-{rng.randrange(1, 30):02d} {rng.randrange(24):02d}:{rng.randrange(60):02d}:{rng.randrange(60):02d}"
    return tpl.format(t=t, n=rng.randrange(1, 20), m=rng.randrange(10, 900), q=rng.choice(names))


def _noise_block(rng: random.Random, lang: str, names: list[str], chars: int) -> str:
    lines = []
    while sum(len(x) + 1 for x in lines) < chars:
        lines.append(_line(rng, lang, names))
    return "\n".join(lines)[:chars]


def _value(rng: random.Random, kind: str, avoid: set | None = None):
    for _ in range(50):
        v = rng.randrange(1000, 9000) if kind == "int" and False else (
            rng.randrange(3000, 9000) if kind == "int" else rng.choice(WORDS)
        )
        if not avoid or v not in avoid:
            return v
    return v


def _slot_text(rng: random.Random, lang: str, names: list[str], fact: str, chars: int, where: str = "any") -> str:
    """사실 한 문장을 소음 속 임의 위치에 넣는다. where=middle이면 본문 한가운데."""
    body = _noise_block(rng, lang, names, chars).split("\n")
    pos = len(body) // 2 if where == "middle" else rng.randrange(len(body) + 1)
    body.insert(pos, fact)
    return "\n".join(body)


def _fmt(tpl: str, **kw) -> str:
    return tpl.format(**kw)


def family(index: int, stratum: str, lang: str, length: str, rng: random.Random) -> dict:
    n_blocks = LENGTHS[length]
    attr = ATTRS[rng.randrange(len(ATTRS))] if stratum != "combine" else ATTRS[0]
    en, ko, kind, syn_en, syn_ko, ev_en, ev_ko = attr
    stem = rng.choice(PROJECT_STEMS)
    project = f"{stem}-{rng.choice(PROJECT_SUFFIX)}-{index:03d}"
    others = [f"{stem}-{s}-{index:03d}x" for s in rng.sample(PROJECT_SUFFIX, 3)]
    names = [f"mod{rng.randrange(99)}" for _ in range(6)]
    value = _value(rng, kind)
    sizes = [rng.randrange(400, 1600) for _ in range(n_blocks)]
    facts: dict[int, str] = {}  # 블록 번호 -> 사실 문장
    support: list[int] = []
    expected: object = value
    ask_en = f'What is the {en} of {project}?'
    ask_ko = f"{project}의 {ko}은(는) 얼마(무엇)인가요?"
    fact_en = _fmt(f"The {en} of {{p}} is {{v}}.", p=project, v=value)
    fact_ko = _fmt(f"{{p}}의 {ko}은(는) {{v}}이다.", p=project, v=value)
    slots = rng.sample(range(n_blocks), 12)
    if stratum == "exact":
        facts[slots[0]] = fact_ko if lang == "ko" else fact_en
        support = [slots[0]]
    elif stratum == "none":
        expected = None
    elif stratum == "other":
        facts[slots[0]] = fact_ko if lang == "ko" else fact_en
        support = [slots[0]]
        for i, q in enumerate(others):
            ov = _value(rng, kind, {value})
            facts[slots[1 + i]] = (fact_ko if lang == "ko" else fact_en).replace(project, q).replace(str(value), str(ov))
    elif stratum == "stale":
        old = _value(rng, kind, {value})
        facts[slots[0]] = (f"{project}의 {ko}은(는) {old}이었다." if lang == "ko" else f"The {en} of {project} was {old}.")
        facts[slots[1]] = (f"갱신: {project}의 {ko}을(를) {value}으로 바꿨다. 이전 값 {old}은 폐기." if lang == "ko"
                           else f"Update: the {en} of {project} changed to {value}; the earlier {old} is obsolete.")
        support = [slots[1]]
    elif stratum == "synonym":
        facts[slots[0]] = _fmt(ev_ko if lang == "ko" else ev_en, p=project, v=value) + "."
        support = [slots[0]]
        ask_en = syn_en.format(p=project).capitalize() + "?"
        ask_ko = syn_ko.format(p=project) + "?"
    elif stratum == "combine":
        host = rng.choice(HOSTS)
        facts[slots[0]] = (f"{project}의 호스트는 {host}이다." if lang == "ko" else f"The host of {project} is {host}.")
        facts[slots[1]] = fact_ko if lang == "ko" else fact_en
        support = [slots[0], slots[1]]
        expected = f"{host}:{value}"
        ask_en = f'Return the host and the port of {project} as a single string "host:port".'
        ask_ko = f'{project}의 호스트와 포트를 "host:port" 한 문자열로 알려 주세요.'
        fact_en = ""
    elif stratum == "middle":
        facts[slots[0]] = fact_ko if lang == "ko" else fact_en
        support = [slots[0]]
        sizes[slots[0]] = 2600 if lang == "ko" else 4200
    # 같은 프로젝트의 다른 속성 사실을 어려운 방해물로 둔다(근거 없음 층 포함)
    for i, oa in enumerate(OTHER_ATTRS[:2]):
        ov = rng.choice(PEOPLE)
        facts[slots[8 + i]] = (f"{project}의 {oa}은(는) {ov}이다." if lang == "ko" else f"The {oa} of {project} is {ov}.")
    # 같은 속성의 다른 프로젝트 방해물(none, stale, synonym 층에도 하나씩)
    if stratum != "other":
        q = others[0]
        ov = _value(rng, kind, {value})
        facts[slots[11]] = (f"{q}의 {ko}은(는) {ov}이다." if lang == "ko" else f"The {en} of {q} is {ov}.")
    blocks = []
    for j in range(n_blocks):
        where = "middle" if (stratum == "middle" and j in support) else "any"
        text = _slot_text(rng, lang, names, facts[j], sizes[j], where) if j in facts else _noise_block(rng, lang, names, sizes[j])
        blocks.append({"id": f"r{j}", "text": text})
    ask = ask_ko if lang == "ko" else ask_en
    task = f'{ask} Reply with only JSON {{"value": <answer or null if the records do not say>}}. Use only the records.'
    if lang == "ko":
        task = f'{ask} 기록에 근거가 없으면 null로, JSON {{"value": ...}} 만 답하세요. 기록만 쓰세요.'
    return dict(
        task_id=f"s{index:03d}", family_id=f"fam541-{index:03d}", stratum=stratum, lang=lang, length=length,
        task=task, blocks=blocks, expected={"value": expected}, support_ids=[f"r{s}" for s in support],
    )


def fixture_hash(case: dict) -> str:
    body = {k: v for k, v in case.items() if k not in ("split", "fixture_hash")}
    return hashlib.sha256(json.dumps(body, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def build() -> list[dict]:
    """개발 계열과 봉인 계열을 한 번에 만든다. 계열 번호가 겹치지 않고 층 x 언어 x 길이 칸마다 같은 수를 둔다."""
    rng = random.Random(SEED)
    cells = [(s, l, n) for s in STRATA for l in LANGS for n in LENGTHS]
    cases = []
    index = 0
    for split, per in (("dev", DEV_PER_CELL), ("sealed", SEALED_PER_CELL)):
        for cell in cells:
            for _ in range(per):
                case = family(index, *cell, rng)
                case["split"] = split
                case["fixture_hash"] = fixture_hash(case)
                cases.append(case)
                index += 1
    return cases


def check(cases: list[dict], prior: set[str]) -> None:
    ids = [c["task_id"] for c in cases]
    hashes = [c["fixture_hash"] for c in cases]
    if len(set(ids)) != len(ids) or len(set(hashes)) != len(hashes):
        raise RuntimeError("duplicate task or fixture")
    texts = [b["text"] for c in cases for b in c["blocks"]]
    if len(set(texts)) != len(texts):
        raise RuntimeError("duplicate block text across families")
    if prior & set(texts):
        raise RuntimeError("block overlaps prior sample")
    for c in cases:
        if fixture_hash(c) != c["fixture_hash"]:
            raise RuntimeError("fixture hash mismatch: " + c["task_id"])
        if len(c["blocks"]) != LENGTHS[c["length"]]:
            raise RuntimeError("block count: " + c["task_id"])
        joined = {b["id"]: b["text"] for b in c["blocks"]}
        if c["stratum"] == "none" and c["support_ids"]:
            raise RuntimeError("none stratum has support")
        if c["stratum"] != "none" and not c["support_ids"]:
            raise RuntimeError("missing support: " + c["task_id"])
        support_text = " ".join(joined[s] for s in c["support_ids"])
        if c["stratum"] != "none" and any(p not in support_text for p in str(c["expected"]["value"]).split(":")):
            raise RuntimeError("support blocks lack the answer: " + c["task_id"])
    per_cell: dict = {}
    for c in cases:
        per_cell.setdefault((c["split"], c["stratum"], c["lang"], c["length"]), 0)
        per_cell[(c["split"], c["stratum"], c["lang"], c["length"])] += 1
    for split, per in (("dev", DEV_PER_CELL), ("sealed", SEALED_PER_CELL)):
        if any(v != per for k, v in per_cell.items() if k[0] == split):
            raise RuntimeError("unbalanced cells")
