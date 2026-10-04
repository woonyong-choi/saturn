"""봉인·원본 불변·예산·분모·크기·응답 계약을 검사한다."""

from __future__ import annotations

import importlib.util
import json
import math
import subprocess
from collections import Counter

from support import BASE, CONDITIONS, PRIVATE, PUBLIC, WORKTREE, read, rows, sha


def require(value: bool, message: str) -> None:
    if not value:
        raise RuntimeError(message)


def local_module(name: str, file: str) -> object:
    spec = importlib.util.spec_from_file_location(name, PUBLIC / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def verify_metrics(summary: dict, analyzer: object, collector: object) -> None:
    baseline = read(PUBLIC.parent / "continuation-judgment-korean/results/summary.json")
    require(
        summary["flow"]["base_labels"] == {"continue": 515, "new": 72, "uncertain": 21},
        "baseline label counts changed",
    )
    require(bool(baseline), "baseline summary missing")
    for r in summary["curves"]:
        require(r["tp"] + r["fn"] == r["recall"]["n"], "recall denominator mismatch")
        require(
            r["fp"] + r["tn"] == r["false_join_rate"]["n"],
            "misjoin denominator mismatch",
        )
        require(r["ask_rate"]["n"] == 608, "ask denominator mismatch")
    require(
        math.isclose(analyzer.exact(0, 72)[1], 1 - 0.0125 ** (1 / 72), abs_tol=1e-10),
        "exact zero-event upper bound wrong",
    )
    require(
        math.isclose(analyzer.exact(72, 72)[0], 0.0125 ** (1 / 72), abs_tol=1e-10),
        "exact all-event lower bound wrong",
    )
    require(
        analyzer.decision(
            {"status": "ok", "answers": {"keep_current": 0.5, "is_new_task": 0.5}}, 0.5
        )
        == "ask",
        "ambiguous boundary accepted",
    )
    require(
        analyzer.decision({"status": "failed"}, 0.8) == "ask", "failure treated as new"
    )
    try:
        collector.judge.parse(
            {"answers": {"keep_current": {"noul": 2}}},
            {"keep_current": {"type": "noul"}},
        )
    except ValueError:
        pass
    else:
        raise RuntimeError("invalid probability accepted")
    require(
        "## 설계와 다른 점" in (PUBLIC / "report.md").read_text(),
        "missing deviation section",
    )
    print(
        json.dumps(
            {
                "verified_curves": len(summary["curves"]),
                "calls": summary["calls"],
                "baseline_unchanged": True,
                "sealed_sources_unchanged": True,
            }
        )
    )


def main() -> None:
    summary = read(PUBLIC / "results/summary.json")
    run = read(PRIVATE / "run.json")
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", run["design_commit"], "HEAD"],
        cwd=WORKTREE,
        check=True,
    )
    for filename in ("design.md", "scripts/01-collect.py", "scripts/support.py"):
        old = subprocess.check_output(
            [
                "git",
                "show",
                f"{run['design_commit']}:docs/experiments/continuation-misjoin/{filename}",
            ],
            cwd=WORKTREE,
        )
        require(old == (PUBLIC / filename).read_bytes(), "sealed source changed")
    for name, digest in run["base_hashes"].items():
        require(sha(BASE / name) == digest, "baseline changed")
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        root, rel = name.split("/", 1)
        require(
            sha((BASE if root == "continuation-ko" else PRIVATE) / rel) == digest,
            "private checksum changed",
        )
    calls, responses = rows(PRIVATE / "calls.jsonl"), rows(PRIVATE / "jev.jsonl")
    counts = Counter(c["kind"] for c in calls)
    require(counts["jev"] <= 4000 and counts["codex"] <= 300, "call budget exceeded")
    require(
        len({r["trial_id"] for r in calls}) == len(calls), "duplicate reserved call"
    )
    require(
        len({r["trial_id"] for r in responses}) == len(responses), "duplicate receipt"
    )
    collector = local_module("verify_collector", "01-collect.py")
    analyzer = local_module("verify_analyzer", "02-analyze.py")
    cases = read(BASE / "sample.json") + read(PRIVATE / "sample.json")
    index = {c["id"]: c for c in cases}
    expected = {
        f"{c['id']}-{b}-r{r}" for c in cases for b in CONDITIONS for r in (1, 2)
    }
    require({r["trial_id"] for r in responses} == expected, "missing scheduled receipt")
    reserved = {r["trial_id"] for r in calls if r["kind"] == "jev"}
    require(
        reserved == {r["trial_id"] for r in responses if r["status"] != "oversize"},
        "call receipt coverage mismatch",
    )
    for r in responses:
        if r["status"] == "incomplete":
            continue
        body = collector.request_for(index[r["id"]], r["condition"])
        require(r["request"] == body, "request differs from sealed condition")
        size = len(json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode())
        require(size == r["request_bytes"], "request byte mismatch")
        require(size <= 100000 or r["status"] == "oversize", "oversized sent request")
        if r["status"] == "ok":
            parsed = collector.judge.parse(
                json.loads(r["raw_response"]), body["questions"]
            )
            require(parsed == r["answers"], "stored probability mismatch")
            require(r["model"] == collector.judge.MODEL, "model mismatch")
    verify_metrics(summary, analyzer, collector)


if __name__ == "__main__":
    main()
