"""실제 기록과 질문을 고정하고 복원 비용을 포함한 대응 실험을 수집한다."""

from __future__ import annotations

from collections import Counter
import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PHASE = os.environ.get("SATURN_RECALL_PHASE", "bounded")
PRIVATE = Path(
    os.environ.get(
        "SATURN_RECALL_HOME", ROOT / ".local/experiments/context-recall" / PHASE
    )
)
BALANCED = PHASE in {"balanced", "bounded"}
BASE = ROOT / ".local/experiments/real-context-replay/corrected"


def load_helper(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(
        name, PUBLIC.parent / "real-context-replay/scripts" / filename
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def question_text(questions):
    return (
        "앞 기록의 근거만으로 답하세요. 도구를 사용하지 마세요. 답이 없으면 unknown입니다. 키별 값이 문자열인 JSON 객체 하나만 답하세요.\n"
        + "\n".join(f"{q['id']}: {q['question']}" for q in questions)
    )


def prepare():
    PRIVATE.mkdir(parents=True, exist_ok=False)
    shutil.copy2(BASE / "cases.json", PRIVATE / "cases.json")
    shutil.copy2(
        PUBLIC / ("questions-balanced.json" if BALANCED else "questions.json"),
        PRIVATE / "questions.json",
    )
    cases = json.loads((PRIVATE / "cases.json").read_text())
    if BALANCED:
        heldout = PRIVATE.parent / "heldout-case.json"
        cases.append(json.loads(heldout.read_text()))
        (PRIVATE / "cases.json").write_text(
            json.dumps(cases, ensure_ascii=False, indent=2)
        )
        shutil.copy2(
            PRIVATE.parent / "heldout-design-source.json",
            PRIVATE / "heldout-design-source.json",
        )
    questions = json.loads((PRIVATE / "questions.json").read_text())
    queries = [
        dict(
            id=c["id"],
            case=c["id"],
            query=question_text(questions[c["cluster"]]),
            max_chars=8000 if BALANCED else 4000,
        )
        for c in cases
    ]
    (PRIVATE / "queries.json").write_text(
        json.dumps(queries, ensure_ascii=False, indent=2)
    )
    env = dict(
        os.environ,
        SATURN_REPLAY_INPUT=str(PRIVATE / "cases.json"),
        SATURN_REPLAY_OUTPUT=str(PRIVATE / "evidence"),
        SATURN_RECALL_QUERIES=str(PRIVATE / "queries.json"),
        CARGO_TARGET_DIR=str(ROOT / "target/cleanup-verification"),
    )
    with (PRIVATE / "replay.log").open("x") as log:
        subprocess.run(
            [
                "cargo",
                "test",
                "-p",
                "saturn-engine",
                "--lib",
                "export_recalled_evidence",
                "--",
                "--ignored",
            ],
            cwd=ROOT,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            check=True,
        )
    if BALANCED:
        env["SATURN_REPLAY_OUTPUT"] = str(PRIVATE / "packets")
        with (PRIVATE / "packet-replay.log").open("x") as log:
            subprocess.run(
                [
                    "cargo",
                    "test",
                    "-p",
                    "saturn-engine",
                    "--lib",
                    "export_real_record_packets",
                    "--",
                    "--ignored",
                ],
                cwd=ROOT,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )
        for source in (BASE / "packets").glob("*.json"):
            if (
                PHASE == "balanced"
                and source.read_bytes()
                != (PRIVATE / "packets" / source.name).read_bytes()
            ):
                raise RuntimeError("original packet baseline changed")
    else:
        shutil.copytree(BASE / "packets", PRIVATE / "packets")
    (PRIVATE / "work").mkdir()
    (PRIVATE / "raw").mkdir()
    snapshot = PRIVATE / "code"
    snapshot.mkdir()
    for source in [
        ROOT / "saturn-terminal/engine/src/handoff_recall.rs",
        ROOT / "saturn-terminal/engine/src/handoff_replay.rs",
        ROOT / "saturn-terminal/engine/src/delivery.rs",
        ROOT / "saturn-terminal/core/src/sessions/ranking.rs",
        PUBLIC / "design.md",
        PUBLIC / "balanced-design.md",
        PUBLIC / "boundary-design.md",
        ROOT / "saturn-terminal/core/src/sessions/packet.rs",
        PUBLIC / "scripts/01-run.py",
        PUBLIC.parent / "real-context-replay/scripts/02-collect.py",
    ]:
        shutil.copy2(source, snapshot / source.name)
    with (snapshot / "working-tree.patch").open("w") as output:
        subprocess.run(["git", "diff", "HEAD"], cwd=ROOT, stdout=output, check=True)
    seal = {
        str(p.relative_to(PRIVATE)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in PRIVATE.rglob("*")
        if p.is_file()
    }
    (PRIVATE / "collection-seal.json").write_text(json.dumps(seal, indent=2))


def collect():
    helper = load_helper("collector", "02-collect.py")
    helper.PRIVATE = PRIVATE
    cases = json.loads((PRIVATE / "cases.json").read_text())
    queries = {
        q["id"]: q["query"] for q in json.loads((PRIVATE / "queries.json").read_text())
    }
    header = (
        json.loads((PRIVATE / "packets/learning-12-4000.json").read_text())[
            "text"
        ].split("\n\n## ", 1)[0]
        + "\n\n"
    )
    reference = json.loads((PRIVATE / "packets/learning-12-4000.json").read_text())[
        "text"
    ]
    footer = "\n\n" + reference.rsplit("\n\n", 1)[-1] if PHASE == "bounded" else ""
    jobs = []
    for repeat in range(1, 2 if BALANCED else 4):
        for case in cases:
            packet = json.loads(
                (PRIVATE / f"packets/{case['id']}-4000.json").read_text()
            )["text"]
            evidence = json.loads(
                (PRIVATE / f"evidence/{case['id']}.json").read_text()
            )["text"]
            for arm in ["full", "packet", "recall"]:
                query = queries[case["id"]]
                if arm == "recall" and evidence:
                    query = (
                        evidence
                        + "\n\nEnd of earlier evidence. Current user input:\n"
                        + query
                    )
                if arm == "recall" and PHASE == "bounded":
                    query = json.loads(
                        (PRIVATE / f"evidence/{case['id']}.json").read_text()
                    )["input"]
                jobs.append(
                    dict(
                        id=f"{case['id']}-r{repeat}",
                        case=case["id"],
                        repeat=repeat,
                        arm=arm,
                        context=header + case["full"] + footer
                        if arm == "full"
                        else packet,
                        question=query,
                    )
                )
    random.Random(7108).shuffle(jobs)
    helper.save(PRIVATE / "calls-plan.json", jobs)
    helper.save(
        PRIVATE / "env.json",
        dict(
            head=subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
            ).strip(),
            versions={
                name: subprocess.check_output([name, "--version"], text=True).strip()
                for name in ["codex", "claude", "rustc"]
            },
            router_key_present=bool(os.environ.get("SATURN_KEY")),
        ),
    )
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(helper.run_provider, provider, jobs)
            for provider in ["claude", "codex"]
        ]
        for future in futures:
            future.result()


def analyze():
    helper = load_helper("analysis", "03-analyze.py")
    questions = json.loads((PRIVATE / "questions.json").read_text())
    cases = {c["id"]: c for c in json.loads((PRIVATE / "cases.json").read_text())}
    records = []
    for provider in ["claude", "codex"]:
        for job in json.loads((PRIVATE / "calls-plan.json").read_text()):
            folder = PRIVATE / "raw" / f"{provider}-{job['id']}-{job['arm']}"
            result = (
                json.loads((folder / "result.json").read_text())
                if (folder / "result.json").exists()
                else dict(
                    status="interrupted"
                    if folder.exists() and (PRIVATE / "stopped.json").exists()
                    else "not_run",
                    provider=provider,
                )
            )
            valid = result["status"] == "ok" and result.get("tool_calls", 0) == 0
            if provider == "claude":
                valid = (
                    valid
                    and result.get("ready") == "Ready"
                    and result.get("same_session") is True
                )
                for raw in [folder / "ready.jsonl", folder / "answer.jsonl"]:
                    if raw.exists():
                        for line in raw.read_text().splitlines():
                            event = json.loads(line)
                            content = event.get("message", {}).get("content", [])
                            if isinstance(content, list) and any(
                                item.get("type") == "tool_use" for item in content
                            ):
                                valid = False
            answer = helper.parsed(result.get("text", ""))
            checks = {
                q["id"]: helper.normalize(answer.get(q["id"]))
                in [helper.normalize(x) for x in q["expected"]]
                for q in questions[cases[job["case"]]["cluster"]]
            }
            records.append(
                dict(
                    provider=provider,
                    case=job["case"],
                    repeat=job["repeat"],
                    arm=job["arm"],
                    valid=valid,
                    status=result["status"],
                    answers=answer,
                    checks=checks,
                    answer_correct=sum(checks.values()),
                    correct=sum(checks.values()) if valid else 0,
                    input_tokens=helper.tokens(result),
                    new_correct=sum(
                        v for k, v in checks.items() if k.startswith("new_")
                    )
                    if valid
                    else 0,
                )
            )
    for record in records:
        qs = questions[cases[record["case"]]["cluster"]]
        record["scores_by_set"] = {
            name: {
                "correct": sum(
                    record["checks"][q["id"]] for q in qs if q["set"] == name
                )
                if record["valid"]
                else 0,
                "questions": sum(q["set"] == name for q in qs),
            }
            for name in sorted({q["set"] for q in qs})
        }
    groups = []
    for provider in ["claude", "codex"]:
        full = sum(
            r["input_tokens"]
            for r in records
            if r["provider"] == provider and r["arm"] == "full"
        )
        for arm in ["full", "packet", "recall"]:
            rows = [r for r in records if r["provider"] == provider and r["arm"] == arm]
            total = sum(r["input_tokens"] for r in rows)
            groups.append(
                dict(
                    provider=provider,
                    arm=arm,
                    correct=sum(r["correct"] for r in rows),
                    answer_correct=sum(r["answer_correct"] for r in rows),
                    new_correct=sum(r["new_correct"] for r in rows),
                    questions=len(rows) * 8,
                    new_questions=sum(
                        r["scores_by_set"].get("new_questions", {}).get("questions", 0)
                        for r in rows
                    ),
                    scores_by_set={
                        name: {
                            metric: sum(
                                r["scores_by_set"].get(name, {}).get(metric, 0)
                                for r in rows
                            )
                            for metric in ["correct", "questions"]
                        }
                        for name in ["development", "new_questions", "heldout_source"]
                    },
                    valid_runs=sum(r["valid"] for r in rows),
                    runs=len(rows),
                    input_tokens=total,
                    reduction=1 - total / full if full else None,
                )
            )
    summary = dict(
        phase=PHASE,
        planned_sessions=len(records),
        sessions_by_status=dict(Counter(r["status"] for r in records)),
        groups=groups,
        records=records,
    )
    output_name = {
        "confirmation": "stopped-summary.json",
        "balanced": "balanced-summary.json",
        "replication": "replication-summary.json",
    }.get(PHASE, "summary.json")
    (PUBLIC / "results" / output_name).write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps(groups, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    os.umask(0o077)
    {"prepare": prepare, "collect": collect, "analyze": analyze}[sys.argv[1]]()
