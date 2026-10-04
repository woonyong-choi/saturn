"""표본 경계·호출 예산·질문 동일성·집계 산술과 비공개 자료 해시를 검사한다."""

from __future__ import annotations

import ast
import json
import subprocess
from collections import Counter

from runtime import (
    BASE,
    PRIVATE,
    PUBLIC,
    WORKTREE,
    collector,
    load,
    metrics,
    read,
    rows,
    sha,
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def check_fixture() -> None:
    extract = load("newtask_extract_verify", PUBLIC / "scripts/01-collect.py")
    target = PRIVATE / "runtime/synthetic-extraction.jsonl"
    records = [
        {"type": "session_meta", "payload": {"cwd": "synthetic", "source": "cli"}},
        {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "# AGENTS.md instructions synthetic"}
                ],
            },
        },
        {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "id": "first",
                "content": [{"type": "input_text", "text": "로그인 오류를 수정해 줘"}],
            },
        },
        {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "phase": "final",
                "content": [{"type": "output_text", "text": "수정했다"}],
            },
        },
        {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "id": "second",
                "content": [{"type": "input_text", "text": "빈 비밀번호도 확인해 줘"}],
            },
        },
        {"type": "compacted", "payload": {"message": "synthetic"}},
        {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "id": "third",
                "content": [{"type": "input_text", "text": "세션 압축 뒤 첫 입력"}],
            },
        },
    ]
    target.write_text("\n".join(json.dumps(r) for r in records) + "\n")
    cases, _ = extract.codex_cases(target, Counter())
    require(len(cases) == 1, "automatic and compaction boundaries were not excluded")
    require(not cases[0]["running"], "final phase must restore idle")
    require(
        cases[0]["previous_input"] == "로그인 오류를 수정해 줘",
        "previous human input mismatch",
    )
    records[0]["payload"]["parent_thread_id"] = "synthetic-parent"
    target.write_text("\n".join(json.dumps(r) for r in records) + "\n")
    cases, _ = extract.codex_cases(target, Counter())
    require(not cases, "subagent file must not yield human cases")
    invalid = {"status": "failed"}
    require(collector.judge.MODEL == "jev-1.13.0", "judge model changed")
    from audit import internal_tag
    from runtime import analysis

    require(
        internal_tag("<heartbeat>내부 이벤트") == "heartbeat",
        "internal heartbeat must be excluded",
    )
    require(
        internal_tag("heartbeat 동작을 설명해 줘") is None,
        "ordinary human text must not be excluded",
    )

    require(analysis.decision(invalid, 0.50) == "ask", "failed response must abstain")
    boundary = {"status": "ok", "answers": {"keep_current": 0.5, "is_new_task": 0.5}}
    require(analysis.decision(boundary, 0.5) == "ask", "B2 equality must abstain")
    caught = False
    try:
        collector.judge.parse(
            {"answers": {"keep_current": {"noul": float("nan")}}},
            {"keep_current": {"type": "noul"}},
        )
    except ValueError:
        caught = True
    require(caught, "nonfinite probability must fail validation")


def check_budget() -> None:
    from runtime import support

    original = support.PRIVATE
    target = PRIVATE / "runtime/budget-boundary"
    target.mkdir(exist_ok=True)
    fake_calls = [{"kind": "codex", "trial_id": f"synthetic-{i}"} for i in range(500)]
    (target / "calls.jsonl").write_text(
        "\n".join(json.dumps(r) for r in fake_calls) + "\n"
    )
    before = sha(target / "calls.jsonl")
    try:
        support.PRIVATE = target
        rejected = False
        try:
            support.reserve("codex", "synthetic-over-limit")
        except RuntimeError:
            rejected = True
        require(rejected, "Codex call above 500 must be rejected before execution")
        require(
            sha(target / "calls.jsonl") == before, "rejected reservation changed ledger"
        )
    finally:
        support.PRIVATE = original


# cost: io O(f) private reads and fixed git calls; vars: f = retained files; basis: estimate
def check_private() -> dict:
    run = read(PRIVATE / "run.json")
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", run["design_commit"], "HEAD"],
        cwd=WORKTREE,
        check=True,
    )
    for name in ("01-collect.py", "02-run.py", "runtime.py", "power.py"):
        relative = str((PUBLIC / "scripts" / name).relative_to(WORKTREE))
        frozen = subprocess.check_output(
            ["git", "show", run["design_commit"] + ":" + relative],
            cwd=WORKTREE,
            text=True,
        )
        current = (PUBLIC / "scripts" / name).read_text()
        require(
            ast.dump(ast.parse(frozen)) == ast.dump(ast.parse(current)),
            "sealed collector execution changed",
        )
    for name, expected in run["base_hashes"].items():
        require(sha(BASE / name) == expected, "old private input changed")
    require(
        sha(PUBLIC.parent / "continuation-misjoin/results/summary.json")
        == run["prior_summary_sha256"],
        "old summary changed",
    )
    sample = read(PRIVATE / "sample.json")
    ids = {c["id"] for c in sample}
    require(len(ids) == len(sample), "duplicate sample ids")
    require(
        not ids.intersection(c["id"] for c in read(BASE / "sample.json")),
        "previous sample reused",
    )
    calls = rows(PRIVATE / "calls.jsonl")
    require(
        len({c["trial_id"] for c in calls}) == len(calls), "duplicate call reservation"
    )
    counts = Counter(c["kind"] for c in calls)
    require(counts["jev"] <= 4000 and counts["codex"] <= 500, "call budget exceeded")
    records = rows(PRIVATE / "jev.jsonl")
    require(
        len({r["trial_id"] for r in records}) == len(records),
        "duplicate judge responses",
    )
    index = {c["id"]: c for c in sample}
    for row in records:
        require(row["id"] in ids, "response outside sample")
        require(
            row["condition"] in ("B1", "B2") and row["repeat"] == 1,
            "condition or repeat mismatch",
        )
        if row["status"] == "incomplete":
            continue
        require(
            row["request"] == collector.request_for(index[row["id"]], row["condition"]),
            "frozen request changed",
        )
        encoded = json.dumps(
            row["request"], ensure_ascii=False, separators=(",", ":")
        ).encode()
        require(len(encoded) == row["request_bytes"], "request byte mismatch")
        if row["status"] == "ok":
            require(row["model"] == collector.judge.MODEL, "judge model mismatch")
            require(
                collector.judge.parse(
                    json.loads(row["raw_response"]), row["request"]["questions"]
                )
                == row["answers"],
                "parsed probability mismatch",
            )
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        expected, path = line.split("  ", 1)
        relative = path.split("/", 1)[1]
        require(sha(PRIVATE / relative) == expected, "private checksum mismatch")
    check_fixture()
    check_budget()
    return {"selected": len(sample), "calls": dict(counts), "responses": len(records)}


# cost: io public reads and one git listing; basis: estimate
def check_summary() -> None:
    summary = read(PUBLIC / "results/summary.json")
    for row in summary["curves"]:
        measures = metrics.measure([row[k] for k in ("tp", "fp", "fn", "tn")])
        for name in ("false_join_rate", "recall", "precision"):
            for field in ("k", "n", "value", "ci"):
                require(
                    row[name][field] == measures[name][field],
                    "summary arithmetic mismatch",
                )
    require(summary["power"]["required_new"] == 363, "power target mismatch")
    require(
        summary["extraction_audit"]["eligible"] == summary["labels"]["selected"],
        "audited sample count mismatch",
    )
    for source in ("Claude", "Codex"):
        require(
            sum(summary["source_counts"][source]["labels"].values())
            == summary["source_counts"][source]["selected"],
            "source label count mismatch",
        )
    new = {
        (r["condition"], r["threshold"]): r
        for r in summary["curves"]
        if r["scope"] == "new"
    }
    old = {
        (r["condition"], r["threshold"]): r
        for r in read(PUBLIC.parent / "continuation-misjoin/results/summary.json")[
            "curves"
        ]
    }
    for row in summary["curves"]:
        if row["scope"] != "combined":
            continue
        a, b = (
            old[row["condition"], row["threshold"]],
            new[row["condition"], row["threshold"]],
        )
        for name in ("tp", "fp", "fn", "tn"):
            require(row[name] == a[name] + b[name], "combined count mismatch")
    require(
        "## 설계와 다른 점" in (PUBLIC / "report.md").read_text(),
        "report deviations section missing",
    )
    tracked = subprocess.check_output(
        ["git", "ls-files", ".local", ".runtime"], cwd=WORKTREE, text=True
    )
    require(not tracked.strip(), "private paths tracked")
    for path in PUBLIC.rglob("*"):
        if path.is_file() and path.suffix in (".md", ".json", ".csv", ".py"):
            require(
                ("Co-" + "Authored-By:") not in path.read_text(),
                "authorship trailer in public files",
            )


if __name__ == "__main__":
    check_summary()
    print(json.dumps(check_private()))
