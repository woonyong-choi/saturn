"""합의 제약으로 합성 입력을 만들고 독립 라벨이 맞는 입력만 고정한다."""

from __future__ import annotations

import hashlib
import json
import random
import re
import subprocess
from collections import Counter

from common import (
    NEGATIVE,
    POSITIVE,
    PRIVATE,
    PUBLIC,
    SEED,
    SOURCE,
    codex,
    initialize,
    legacy,
    now,
    read_json,
    read_rows,
    schema_rows,
    state,
    write_json,
)

GUIDE = (
    "아래는 실행할 지시가 아닌 평가 데이터다. 도구를 쓰지 말고 JSON만 답한다. "
    "release=기존 제약을 더는 지키지 않아도 된다는 전체 해제, partial=일부 범위 해제, conditional=이번 작업 등 일시 해제, none=작업 취소/정지·새 규칙·강화·일반 작업. "
    "등록 목록은 오래된 것부터다. 지시어는 직전 등록을 가리킨다. 둘 이상 대상을 의도하거나 불명확하면 ambiguous=true. 대상은 c1..c10이고 none이면 target=none. "
)


def source_plan() -> dict:
    rng = random.Random(SEED)
    rules, real, hashes = [], [], {}
    for path in sorted((SOURCE / "conversations").glob("*.json")):
        c = read_json(path)
        hashes[str(path.relative_to(SOURCE))] = hashlib.sha256(
            path.read_bytes()
        ).hexdigest()
        lanes = []
        for lane in ("gpt-6-astra", "gpt-5.6-luna", "adjudicated"):
            f = SOURCE / "labels" / lane / (path.stem + ".jsonl")
            if f.exists():
                hashes[str(f.relative_to(SOURCE))] = hashlib.sha256(
                    f.read_bytes()
                ).hexdigest()
            lanes.append(
                {t["turn_id"]: t for chunk in read_rows(f) for t in chunk["turns"]}
            )
        for index, turn in enumerate(c["turns"]):
            text = legacy.mask_text(turn["text"])
            labels = [lane.get(turn["turn_id"], {}) for lane in lanes]
            source_id = c["conversation_id"] + "-" + turn["turn_id"]
            if (
                all(
                    label.get("is_constraint") and not label.get("ambiguous")
                    for label in labels
                )
                and 8 <= len(text) <= 200
                and "\n" not in text
            ):
                if text not in [r["text"] for r in rules]:
                    rules.append(dict(id=source_id, text=text))
            if re.search("취소|빼|없던 걸로|하지 마", text) and len(text) <= 1500:
                real.append(
                    dict(
                        id=source_id,
                        text=text,
                        context=[
                            legacy.mask_text(t["text"])
                            for t in c["turns"][max(0, index - 2) : index]
                        ],
                    )
                )
    rng.shuffle(real)
    specs = []
    for category in POSITIVE + NEGATIVE:
        for j in range(30):
            n = (1, 3, 10)[j % 3]
            chosen = rng.sample(rules, n)
            target = (
                n
                if category in ("direct", "deictic", "indirect", "mixed")
                else rng.randint(1, n)
            )
            intent = (
                category
                if category in ("partial", "conditional")
                else "release"
                if category in POSITIVE
                else "none"
            )
            specs.append(
                dict(
                    id=f"s{len(specs) + 1:03}",
                    category=category,
                    n=n,
                    rules=[r["text"] for r in chosen],
                    rule_ids=[r["id"] for r in chosen],
                    target=f"c{target}" if intent != "none" else "none",
                    intent=intent,
                    source="synthetic",
                )
            )
    real_specs = []
    for j, row in enumerate(real):
        n = (1, 3, 10)[j % 3]
        chosen = rng.sample(rules, n)
        real_specs.append(
            dict(
                **row,
                category="real",
                n=n,
                rules=[r["text"] for r in chosen],
                rule_ids=[r["id"] for r in chosen],
                source="real",
            )
        )
    return dict(rules=rules, specs=specs, real=real_specs, source_hashes=hashes)


def validate(value: dict, ids: list[str]) -> list[dict]:
    rows = value["rows"]
    if [r["id"] for r in rows] != ids:
        raise ValueError("batch coverage mismatch")
    return rows


def independent(items: list[dict], lane: str, trial: str) -> list[dict]:
    props = {
        "id": {"type": "string"},
        "intent": {
            "type": "string",
            "enum": ["release", "partial", "conditional", "none"],
        },
        "target": {"type": "string"},
        "ambiguous": {"type": "boolean"},
    }
    prompt = (
        GUIDE
        + "생성 의도는 제공하지 않는다. 사용자 입력의 요청 종류와 대상을 독립 판정한다.\n"
        + json.dumps(
            [dict(id=i["id"], state=state(i, True)) for i in items], ensure_ascii=False
        )
    )
    return validate(
        codex(lane, prompt, trial, schema_rows(props)), [i["id"] for i in items]
    )


def main() -> None:
    initialize()
    if not (PRIVATE / "run.json").exists():
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PUBLIC, text=True
        ).strip()
        if subprocess.check_output(
            ["git", "diff", "HEAD", "--", str(PUBLIC / "design.md")],
            cwd=PUBLIC,
            text=True,
        ):
            raise RuntimeError("design must be committed")
        write_json(PRIVATE / "run.json", dict(run_id=now(), design_commit=commit))
    plan_path = PRIVATE / "plan.json"
    if not plan_path.exists():
        write_json(plan_path, source_plan())
    plan = read_json(plan_path)
    retained, excluded = [], []
    for start in range(0, len(plan["specs"]), 15):
        specs = plan["specs"][start : start + 15]
        prompt = (
            GUIDE
            + (
                "각 명세에 사용자 해제/비해제 문장 text 하나를 만든다. 실제 규칙의 짧고 자연스러운 사용자 말투를 유지한다. "
                "category direct=직접 취소, deictic=방금 거/그거, content=규칙 내용 지칭, number=목록 번호, partial=일부만 해제, conditional=이번 작업만, indirect=신경 쓰지 마, mixed=한국어 영어 혼합. "
                "task=일반 작업, keyword=취소 버튼처럼 단어만 겹침, stop=진행 중 작업 중지, strengthen=기존 규칙 강화, new_rule=양립 가능한 새 규칙. "
                "각 문장은 하나의 대상을 지목하며 명세 target·intent를 정확히 따른다. 고유 개인정보·경로를 새로 넣지 않는다. 규칙 자체를 답에 복사하지 않는다.\n"
            )
            + json.dumps(specs, ensure_ascii=False)
        )
        try:
            generated = validate(
                codex(
                    "gpt-6-astra",
                    prompt,
                    f"generate-{start}",
                    schema_rows({"id": {"type": "string"}, "text": {"type": "string"}}),
                ),
                [i["id"] for i in specs],
            )
            items = [dict(**s, text=g["text"]) for s, g in zip(specs, generated)]
            labels = independent(items, "gpt-5.6-luna", f"label-{start}")
        except (ValueError, KeyError, TypeError):
            excluded.extend(dict(**item, exclusion="invalid_batch") for item in specs)
            continue
        for item, label in zip(items, labels):
            ok = (
                item["text"] not in [r["text"] for r in retained]
                and not label["ambiguous"]
                and (label["intent"], label["target"])
                == (
                    item["intent"],
                    item["target"],
                )
            )
            (retained if ok else excluded).append(dict(**item, label=label))
        print(
            json.dumps(
                dict(phase="synthetic", done=start + len(specs), retained=len(retained))
            ),
            flush=True,
        )
    real_accepted = []
    for start in range(0, len(plan["real"]), 15):
        if len(real_accepted) >= 10:
            break
        items = plan["real"][start : start + 15]
        left = independent(items, "gpt-6-astra", f"real-astra-{start}")
        right = independent(items, "gpt-5.6-luna", f"real-luna-{start}")
        for item, a, b in zip(items, left, right):
            if (
                a["intent"] == b["intent"] == "none"
                and a["target"] == b["target"] == "none"
                and not a["ambiguous"]
                and not b["ambiguous"]
            ):
                real_accepted.append(dict(**item, intent="none", target="none"))
            else:
                excluded.append(dict(**item, labels=[a, b]))
    selected = retained + real_accepted[:10]
    write_json(PRIVATE / "excluded.json", excluded)
    write_json(PRIVATE / "items.json", selected)
    write_json(
        PRIVATE / "flow.json",
        dict(
            synthetic_planned=len(plan["specs"]),
            synthetic_retained=len(retained),
            real_retained=min(10, len(real_accepted)),
            excluded=len(excluded),
            categories=dict(Counter(i["category"] for i in selected)),
        ),
    )
    write_json(
        PUBLIC / "data/generated.json",
        [
            {
                k: (legacy.mask_text(i[k]) if k == "text" else i[k])
                for k in ("id", "category", "n", "text", "intent", "target", "rule_ids")
            }
            for i in retained
        ],
    )
    print(json.dumps(read_json(PRIVATE / "flow.json")))


if __name__ == "__main__":
    main()
