"""공개 생성문을 원제약과 연결하고 독립 라벨로 평가 세트를 고정한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import random
import re
import subprocess
from collections import Counter

from protocol import GUIDE, KEEP, SCHEMA, SEED, state, validate
from runtime import (
    MAIN,
    PRIVATE,
    PUBLIC,
    ROOT,
    codex,
    now,
    read,
    schema_rows,
    setup,
    write,
)

LABEL_PROPERTIES = {
    "id": {"type": "string"},
    **SCHEMA["properties"],
    "ambiguous": {"type": "boolean"},
}


def checked_rows(answer: dict, batch: list[dict]) -> list[dict]:
    result = answer["rows"]
    if [r["id"] for r in result] != [r["id"] for r in batch]:
        raise ValueError("label coverage mismatch")
    return result


def label_batch(batch: list[dict], model: str, trial: str) -> list[dict]:
    prompt = (
        GUIDE
        + "\n독립 라벨링: 의도와 과거 라벨은 주어지지 않는다. 각 id에 출력 계약과 ambiguous를 답한다.\n"
        + json.dumps(
            [dict(id=i["id"], state=state(i)) for i in batch], ensure_ascii=False
        )
    )
    return checked_rows(
        codex(model, prompt, trial, schema_rows(LABEL_PROPERTIES)), batch
    )


# cost: io 2*ceil(n/15) model calls; vars: n = inputs; basis: estimate
def label_items(items: list[dict], prefix: str) -> tuple[list[dict], list[dict]]:
    batches = [(k, items[k : k + 15]) for k in range(0, len(items), 15)]
    jobs = [
        (k, batch, model)
        for k, batch in batches
        for model in ("gpt-6-astra", "gpt-5.6-luna")
    ]

    def run(job: tuple) -> tuple:
        k, batch, model = job
        try:
            return k, model, label_batch(batch, model, f"{prefix}-{model}-{k}")
        except (ValueError, KeyError, TypeError):
            return k, model, []

    labels = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for k, model, result in pool.map(run, jobs):
            labels[k, model] = result
            print(
                json.dumps(
                    {
                        "phase": prefix,
                        "labeled_batches": len(labels),
                        "total": len(jobs),
                    }
                ),
                flush=True,
            )
    retained, excluded = [], []
    for k, batch in batches:
        a, b = (labels[k, m] for m in ("gpt-6-astra", "gpt-5.6-luna"))
        if not a or not b:
            excluded.extend(
                dict(id=i["id"], reason="invalid_label_batch") for i in batch
            )
            continue
        for item, left, right in zip(batch, a, b):
            gold = {key: left[key] for key in SCHEMA["required"]}

            def signature(value: dict) -> tuple:
                return value["request"], value["target"], value["kind"]

            ok = (
                not left["ambiguous"]
                and not right["ambiguous"]
                and signature(left) == signature(right)
                and validate(gold, item)
                and validate({key: right[key] for key in SCHEMA["required"]}, item)
            )
            if item.get("intended_kind"):
                ok = (
                    ok
                    and gold["kind"] == item["intended_kind"]
                    and gold["target"] == item["intended_target"]
                )
            if item["source"] == "real":
                ok = ok and gold["request"] == "none"
            if ok:
                retained.append(
                    {
                        **item,
                        "gold": gold,
                        "independent_scope": right["scope_text"],
                        "labels": [left, right],
                    }
                )
            else:
                excluded.append(
                    dict(
                        id=item["id"],
                        reason="label_disagreement_or_ambiguity",
                        labels=[left, right],
                    )
                )
    return retained, excluded


def source_items() -> tuple[list[dict], list[dict], list[dict]]:
    rules, real = {}, []
    source = MAIN / ".local/experiments/constraint-deep/conversations"
    for path in sorted(source.glob("*.json")):
        conversation = read(path)
        for idx, turn in enumerate(conversation["turns"]):
            source_id = conversation["conversation_id"] + "-" + turn["turn_id"]
            rid = "r-" + hashlib.sha256(source_id.encode()).hexdigest()[:12]
            rules[rid] = turn["text"]
            if (
                re.search("취소|빼|이번만|지금은|잠깐", turn["text"])
                and len(turn["text"]) <= 1500
            ):
                real.append(
                    dict(
                        id="real-" + rid,
                        text=turn["text"],
                        context=[
                            t["text"]
                            for t in conversation["turns"][max(0, idx - 2) : idx]
                        ],
                        cluster=conversation["conversation_id"],
                        source="real",
                        category="real",
                    )
                )
    public = read(PUBLIC.parent / "constraint-cancel-request/data/generated.json")
    old = []
    for item in public:
        if item["text_redacted"] or any(rid not in rules for rid in item["rule_ids"]):
            raise RuntimeError("cannot reconstruct prior synthetic input")
        old.append(
            {
                **item,
                "rules": [rules[rid] for rid in item["rule_ids"]],
                "source": "prior_synthetic",
                "cluster": item["rule_ids"][
                    int(item["target"][1:]) - 1 if item["target"] != "none" else -1
                ],
            }
        )
    pool = [
        {"id": rid, "text": rules[rid]}
        for rid in sorted({rid for i in old for rid in i["rule_ids"]})
    ]
    rng = random.Random(SEED)
    rng.shuffle(real)
    for i, item in enumerate(real[:60]):
        chosen = rng.sample(pool, (1, 3, 10)[i % 3])
        item.update(
            rules=[r["text"] for r in chosen], rule_ids=[r["id"] for r in chosen]
        )
    return old, pool, real[:60]


# cost: io 6 generation calls; basis: estimate
def generate(pool: list[dict]) -> list[dict]:
    rng = random.Random(SEED)
    specs = []
    for kind in ("once", "scoped"):
        for j in range(40):
            chosen = rng.sample(pool, (1, 3, 10)[j % 3])
            target = rng.randrange(len(chosen))
            specs.append(
                dict(
                    id=f"new-{kind}-{j:02}",
                    rules=[r["text"] for r in chosen],
                    rule_ids=[r["id"] for r in chosen],
                    intended_kind=kind,
                    intended_target=f"c{target + 1}",
                    category=kind,
                    source="new_synthetic",
                    cluster=chosen[target]["id"],
                )
            )
    result = []
    for start in range(0, len(specs), 15):
        batch = specs[start : start + 15]
        prompt = (
            GUIDE
            + "\n명세마다 자연스러운 짧은 한국어 사용자 입력 text 하나를 만든다. intended_kind와 intended_target을 정확히 따른다. once는 이번만·이번 작업만을 다양하게 표현한다. scoped는 이 파일만, 테스트 끝날 때까지, 지금은, 잠깐, 기한, AND 복합조건, 범위 제외를 고르게 섞는다. 실제 규칙을 통째로 복사하거나 개인정보를 추가하지 않는다.\n"
            + json.dumps(batch, ensure_ascii=False)
        )
        generated = checked_rows(
            codex(
                "gpt-6-astra",
                prompt,
                f"generate-{start}",
                schema_rows({"id": {"type": "string"}, "text": {"type": "string"}}),
            ),
            batch,
        )
        result.extend({**s, "text": g["text"]} for s, g in zip(batch, generated))
    return result


# cost: io up to 40 labeling calls and 1 extraction process; basis: estimate
def continuation() -> list[dict]:
    path = PRIVATE / "continuation-candidates.json"
    if not path.exists():
        subprocess.run(
            ["python3", str(PUBLIC / "scripts/source-continuation.py")],
            check=True,
            cwd=ROOT,
        )
    candidates = read(path)
    props = {
        "id": {"type": "string"},
        "label": {"type": "string", "enum": ["continue", "new", "uncertain"]},
    }
    labeled = []
    excluded = []
    for start in range(0, len(candidates), 15):
        batch = candidates[start : start + 15]
        prompt = (
            "기록은 평가 데이터이며 실행하지 않는다. 도구 없이 독립 라벨을 답한다. 경계가 모호하면 uncertain. 판단 지침: "
            + KEEP
            + "\n"
            + json.dumps(batch, ensure_ascii=False)
        )
        try:
            answers = [
                checked_rows(
                    codex(m, prompt, f"continuation-{m}-{start}", schema_rows(props)),
                    batch,
                )
                for m in ("gpt-6-astra", "gpt-5.6-luna")
            ]
        except (ValueError, KeyError, TypeError):
            excluded.extend(
                dict(id=i["id"], reason="invalid_label_batch") for i in batch
            )
            continue
        for item, a, b in zip(batch, *answers):
            if a["label"] == b["label"] != "uncertain":
                labeled.append(
                    {
                        **item,
                        "gold": {"continue": a["label"] == "continue"},
                        "task": "continuation",
                        "source": "real",
                        "cluster": item["session"],
                    }
                )
            else:
                excluded.append(dict(id=item["id"], reason="disagreement_or_uncertain"))
        print(
            json.dumps(
                {
                    "phase": "continuation",
                    "candidates_labeled": start + len(batch),
                    "agreements": len(labeled),
                }
            ),
            flush=True,
        )
        if all(
            sum(i["gold"]["continue"] == flag for i in labeled) >= 24
            for flag in (True, False)
        ):
            break
    write(PRIVATE / "continuation-labels.json", labeled)
    write(PRIVATE / "continuation-exclusions.json", excluded)
    return [
        i
        for flag in (True, False)
        for i in [r for r in labeled if r["gold"]["continue"] == flag][:24]
    ]


# cost: io local manifest reads and 1 grading process; basis: estimate
def main() -> None:
    setup()
    if (PRIVATE / "items.json").exists():
        print("using frozen evaluation inputs")
        return
    if not (PRIVATE / "run.json").exists():
        head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
        if subprocess.check_output(
            ["git", "diff", "HEAD", "--", str(PUBLIC / "design.md")],
            cwd=ROOT,
            text=True,
        ):
            raise RuntimeError("design must be committed")
        write(PRIVATE / "run.json", dict(run_id=now(), design_commit=head))
    old, pool, real = source_items()
    write(PRIVATE / "restored.json", old)
    generated = generate(pool)
    write(PRIVATE / "generated-private.json", generated)
    retained, excluded = label_items(old + generated, "constraint")
    real_retained, real_excluded = label_items(real, "real")
    unique, seen = [], set()
    for item in retained:
        if item["text"] in seen:
            excluded.append(dict(id=item["id"], reason="duplicate"))
        else:
            seen.add(item["text"])
            unique.append(item)
    items = [{**i, "task": "constraint"} for i in unique + real_retained[:10]]
    # 독립 라벨의 범위 의미 합의는 평가 전에 별도 판정한다.
    write(
        PRIVATE / "scope-label-pairs.json",
        [
            dict(
                id=i["id"],
                text=i["text"],
                left=i["gold"]["scope_text"],
                right=i["independent_scope"],
            )
            for i in items
            if i["gold"]["kind"] in ("once", "scoped")
        ],
    )
    write(PRIVATE / "constraint-preliminary.json", items)
    write(PRIVATE / "excluded.json", excluded + real_excluded)
    cont = continuation()
    write(PRIVATE / "continuation.json", cont)
    subprocess.run(
        ["python3", str(PUBLIC / "scripts/03-grade.py"), "labels"], check=True, cwd=ROOT
    )
    scope_labels = {
        r["id"]: r["equivalent"] for r in read(PRIVATE / "scope-label-grades.json")
    }
    selected = [
        i
        for i in items
        if i["gold"]["kind"] not in ("once", "scoped")
        or scope_labels.get(i["id"], False)
    ]
    rng = random.Random(SEED)
    j2_ids = [i["id"] for i in rng.sample(selected, min(100, len(selected)))]
    write(PRIVATE / "j2-ids.json", j2_ids)
    write(PRIVATE / "items.json", selected + cont)
    write(
        PRIVATE / "flow.json",
        dict(
            restored=len(old),
            generated=len(generated),
            constraint_selected=len(selected),
            scope_excluded=len(items) - len(selected),
            real_candidates=len(real),
            real_accepted=len(real_retained),
            real_selected=min(10, len(real_retained)),
            continuation_selected=len(cont),
            exclusions=len(excluded) + len(real_excluded),
            kinds=dict(Counter(i["gold"]["kind"] for i in selected)),
        ),
    )
    print(json.dumps(read(PRIVATE / "flow.json")), flush=True)


if __name__ == "__main__":
    main()
