"""조건 이름을 가린 범위 문장 쌍을 고정 지침으로 판정한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import sys

from exploration import decimal_sensitivity, unwrap
from protocol import SEMANTIC_GUIDE
from runtime import PRIVATE, codex, read, schema_rows, setup, write

PROPERTIES = {
    "id": {"type": "string"},
    "equivalent": {"type": "boolean"},
    "reason": {"type": "string"},
}


# cost: io ceil(n/15) model calls; vars: n = distinct scope pairs; basis: estimate
def grade(pairs: list[dict], prefix: str) -> list[dict]:
    jobs = [(k, pairs[k : k + 15]) for k in range(0, len(pairs), 15)]

    def run(job: tuple) -> list[dict]:
        k, batch = job
        prompt = SEMANTIC_GUIDE + "\n" + json.dumps(batch, ensure_ascii=False)
        reply = codex("gpt-6-astra", prompt, f"{prefix}-{k}", schema_rows(PROPERTIES))[
            "rows"
        ]
        if [r["id"] for r in reply] != [r["id"] for r in batch]:
            raise RuntimeError("scope grading coverage mismatch")
        return reply

    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for result in pool.map(run, jobs):
            results.extend(result)
            print(
                json.dumps(
                    {"grade": prefix, "done": len(results), "total": len(pairs)}
                ),
                flush=True,
            )
    return results


def pair_id(item_id: str, scope: str) -> str:
    return hashlib.sha256((item_id + "\0" + scope).encode()).hexdigest()[:24]


def main() -> None:
    setup()
    if len(sys.argv) > 1 and sys.argv[1] == "labels":
        write(
            PRIVATE / "scope-label-grades.json",
            grade(read(PRIVATE / "scope-label-pairs.json"), "scope-label"),
        )
        return
    items = {i["id"]: i for i in read(PRIVATE / "items.json")}
    rounding = len(sys.argv) > 1 and sys.argv[1] == "rounding"
    existing = (
        {r["id"] for r in read(PRIVATE / "scope-prediction-grades.json")}
        if rounding
        else set()
    )
    pairs = {}
    for path in sorted((PRIVATE / "workflows").glob("*.json")):
        row = read(path)
        item = items[row["item_id"]]
        if rounding:
            row = decimal_sensitivity(row, item)
            if not row["exploratory_decimal"]:
                continue
        if row["condition"] == "L1":
            row = unwrap(row, item)
        if (
            item["task"] != "constraint"
            or item["gold"]["kind"] not in ("once", "scoped")
            or row["status"] != "ok"
        ):
            continue
        scope = row["prediction"]["scope_text"]
        if scope is None:
            continue
        pid = pair_id(item["id"], scope)
        if pid in existing:
            continue
        pairs[pid] = dict(
            id=pid, text=item["text"], left=item["gold"]["scope_text"], right=scope
        )
    prefix = "scope-rounding" if rounding else "scope-prediction"
    write(PRIVATE / (prefix + "-pairs.json"), list(pairs.values()))
    write(
        PRIVATE / (prefix + "-grades.json"),
        grade(list(pairs.values()), prefix),
    )


if __name__ == "__main__":
    main()
