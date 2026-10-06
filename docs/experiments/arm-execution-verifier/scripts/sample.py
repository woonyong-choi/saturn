"""독립 계열 표본을 만들고 개발·봉인 평가로 나눈다. 정답은 요청 상태와 분리한다."""

from __future__ import annotations

import hashlib
import json
import random

from arm_runtime import JEV_SCRIPTS, SEED, load_script

BLOCKS = 12
BLOCK_BYTES = 512
FAMILIES = 24
SUPPORT = 3


def fixture_hash(case: dict) -> str:
    body = json.dumps(case, sort_keys=True, ensure_ascii=False).encode()
    return hashlib.sha256(body).hexdigest()


def split_of(family_id: str) -> str:
    """계열 단위로 개발 20%와 봉인 평가 80%를 나눈다."""
    return "dev" if int(hashlib.sha256(family_id.encode()).hexdigest()[:8], 16) % 5 == 0 else "sealed"


def _block(text: str) -> str:
    return (text + " archival padding." * 40).encode()[:BLOCK_BYTES].decode()


def family(index: int, rng: random.Random) -> list[dict]:
    """한 계열은 같은 기록 12블록 위의 두 과제(전체 설정, 갱신된 포트)다."""
    project = f"fam540-{index:03d}"
    region = rng.choice(["seoul", "osaka", "oslo", "lima"])
    port = rng.randrange(4000, 9000)
    new_port = port + rng.randrange(1, 90)
    token = "".join(rng.choices("abcdef0123456789", k=12))
    missing = index % 4 == 3
    slots = rng.sample(range(BLOCKS), 4)
    facts = [
        f"Current region for {project} is {region}.",
        f"Port for {project} was {port}.",
        f"Current token for {project} is {token}."
        if not missing
        else f"The token for {project} was never recorded.",
        f"Update: port for {project} changed to {new_port}; the earlier port is obsolete.",
    ]
    blocks = []
    for j in range(BLOCKS):
        text = f"Settings notes for unrelated-{index}-{j}: unrelated service, ignore for {project}. old token deadbeef."
        if j in slots:
            text = facts[slots.index(j)]
        blocks.append({"id": f"b{j}", "text": _block(text)})
    support = [f"b{s}" for s in slots]
    return [
        {
            "task_id": f"x{index:03d}a",
            "family_id": project,
            "task": f'Return current deployment settings for {project} as JSON {{"region":string,"port":integer,"token":string or null}}. Use only records, apply updates, null for absent values.',
            "blocks": blocks,
            "expected": {
                "region": region,
                "port": new_port,
                "token": None if missing else token,
            },
            "support_ids": support,
        },
        {
            "task_id": f"x{index:03d}b",
            "family_id": project,
            "task": f'Return only the current port for {project} as JSON {{"port":integer}}. Use only records and apply updates.',
            "blocks": blocks,
            "expected": {"port": new_port},
            "support_ids": [support[1], support[3]],
        },
    ]


def build(families: int = FAMILIES) -> list[dict]:
    rng = random.Random(SEED)
    cases = []
    for i in range(families):
        for case in family(i, rng):
            case["split"] = split_of(case["family_id"])
            case["fixture_hash"] = fixture_hash(
                {k: v for k, v in case.items() if k != "split"}
            )
            cases.append(case)
    return cases


def prior_block_texts() -> set[str]:
    """이전 실험의 맥락 표본과 겹치는지 확인할 기록 원문."""
    fixtures = load_script(JEV_SCRIPTS / "fixtures.py", "jev_role_fixtures")
    return {b["text"] for c in fixtures.contexts() for b in c["blocks"]}


def check_independence(cases: list[dict], prior: set[str]) -> None:
    """계열 누수, 중복, 이전 표본 겹침, 요청에 정답 누출 가능성을 막는다."""
    splits: dict[str, set[str]] = {}
    for c in cases:
        splits.setdefault(c["family_id"], set()).add(c["split"])
    if any(len(v) != 1 for v in splits.values()):
        raise RuntimeError("family appears in both splits")
    hashes = [c["fixture_hash"] for c in cases]
    if len(set(hashes)) != len(hashes):
        raise RuntimeError("duplicate fixture")
    ids = [c["task_id"] for c in cases]
    if len(set(ids)) != len(ids):
        raise RuntimeError("duplicate task id")
    for c in cases:
        if fixture_hash({k: v for k, v in c.items() if k != "split" and k != "fixture_hash"}) != c["fixture_hash"]:
            raise RuntimeError("fixture hash mismatch: " + c["task_id"])
        if len(c["blocks"]) != BLOCKS or any(
            len(b["text"].encode()) != BLOCK_BYTES for b in c["blocks"]
        ):
            raise RuntimeError("unequal block size: " + c["task_id"])
        if prior & {b["text"] for b in c["blocks"]}:
            raise RuntimeError("block overlaps prior sample: " + c["task_id"])
