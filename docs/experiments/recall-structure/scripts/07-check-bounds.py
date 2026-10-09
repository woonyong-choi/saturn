"""실제 자료와 고정 예산 행렬로 글자 상한과 출처 경계를 검사한다."""

import json
import os
from pathlib import Path
import re
import subprocess

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-structure"
RUN = PRIVATE / "formal"
OUTPUT = PRIVATE / "boundary-matrix"
BUDGETS = [0, 1, 126, 127, 128, 256, 512, 1000, 2000, 4000, 8000, 16000]


def load(path):
    return json.loads(path.read_text())


def run():
    OUTPUT.mkdir(exist_ok=False)
    queries = [
        dict(q, id=f"{q['id']}-{budget}", max_chars=budget)
        for q in load(RUN / "queries.json")
        for budget in BUDGETS
    ]
    (OUTPUT / "queries.json").write_text(
        json.dumps(queries, ensure_ascii=False, indent=2) + "\n"
    )
    cases = {c["id"]: c for c in load(RUN / "cases.json")}
    checks = []
    binaries = {
        "baseline": PRIVATE / "baseline/engine-tests",
        "repaired": PRIVATE / "candidate-engine-tests",
    }
    for arm, binary in binaries.items():
        env = dict(os.environ)
        env.update(
            SATURN_REPLAY_INPUT=str(RUN / "cases.json"),
            SATURN_RECALL_QUERIES=str(OUTPUT / "queries.json"),
            SATURN_REPLAY_OUTPUT=str(OUTPUT / arm),
        )
        with (OUTPUT / f"{arm}.log").open("w") as log:
            subprocess.run(
                [
                    str(binary),
                    "handoff::replay::export_recalled_evidence",
                    "--ignored",
                    "--exact",
                ],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )
        for query in queries:
            text = load(OUTPUT / arm / f"{query['id']}.json")["text"]
            sources = {
                (str(r["seq"]), str(r["session"])) for r in cases[query["case"]]["rows"]
            }
            found = re.findall(
                r"\[record (\d+), session (\d+), (?:User(?: steer)?|Agent|Tool)\]",
                text or "",
            )
            checks.append(
                dict(
                    arm=arm,
                    case=query["case"],
                    budget=query["max_chars"],
                    chars=len(text or ""),
                    bounded=len(text or "") <= query["max_chars"],
                    has_source=text is None or bool(found),
                    valid_sources=all(source in sources for source in found),
                )
            )
    result = dict(
        checks=checks,
        exports=len(checks),
        budgets=BUDGETS,
        passed=all(
            c["bounded"] and c["has_source"] and c["valid_sources"] for c in checks
        ),
        scope="character bounds and record/session provenance, not answer quality",
    )
    (PUBLIC / "results/boundary-verification.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )
    print(json.dumps({k: v for k, v in result.items() if k != "checks"}))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    run()
