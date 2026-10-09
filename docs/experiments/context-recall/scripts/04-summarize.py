"""각 단계의 재계산 결과와 검증 로그를 보고서의 단일 수치 원본으로 묶는다."""

from __future__ import annotations

from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/context-recall"
CHECKS = ROOT / ".local/verification/context-recall"


def read(path):
    return json.loads(path.read_text())


def test_totals(path):
    matches = re.findall(
        r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored",
        path.read_text(),
    )
    if not matches or any(row[0] != "ok" for row in matches):
        raise RuntimeError(f"tests did not all pass: {path}")
    return dict(
        passed=sum(int(row[1]) for row in matches),
        failed=sum(int(row[2]) for row in matches),
        ignored=sum(int(row[3]) for row in matches),
        log_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
    )


def main():
    for phase in ["confirmation", "balanced", "bounded", "replication"]:
        with (CHECKS / f"{phase}-recomputed.json").open("w") as log:
            subprocess.run(
                ["python3", str(PUBLIC / "scripts/01-run.py"), "analyze"],
                cwd=ROOT,
                env=dict(os.environ, SATURN_RECALL_PHASE=phase),
                stdout=log,
                check=True,
            )
    summary = read(PUBLIC / "results/summary.json")
    stopped = read(PUBLIC / "results/stopped-summary.json")
    balanced = read(PUBLIC / "results/balanced-summary.json")
    repeated = read(PUBLIC / "results/replication-summary.json")
    summary["earlier_phases"] = {
        name: {
            key: value[key]
            for key in ["planned_sessions", "sessions_by_status", "groups"]
        }
        for name, value in [("confirmation", stopped), ("balanced", balanced)]
    }
    summary["replication"] = {
        key: repeated[key]
        for key in ["planned_sessions", "sessions_by_status", "groups"]
    }
    recall = [
        row
        for row in summary["records"] + repeated["records"]
        if row["arm"] == "recall"
    ]
    summary["combined_recall"] = [
        dict(
            provider=provider,
            correct=sum(
                row["correct"] for row in recall if row["provider"] == provider
            ),
            questions=sum(
                len(row["checks"]) for row in recall if row["provider"] == provider
            ),
            runs=sum(row["provider"] == provider for row in recall),
        )
        for provider in ["claude", "codex"]
    ]
    summary["remaining_failures"] = [
        dict(
            provider=row["provider"],
            case=row["case"],
            repeat=row["repeat"],
            valid=row["valid"],
            incorrect={
                key: row["answers"].get(key)
                for key, correct in row["checks"].items()
                if not correct
            },
        )
        for row in recall
        if row["correct"] != len(row["checks"])
    ]
    all_records = (
        stopped["records"]
        + balanced["records"]
        + summary["records"]
        + repeated["records"]
    )
    summary["all_sessions_by_status"] = dict(
        Counter(row["status"] for row in all_records)
    )
    cases = read(PRIVATE / "bounded/cases.json")
    summary["source_profile"] = dict(
        cases=len(cases),
        clusters=len({c["cluster"] for c in cases}),
        questions_per_condition=sum(
            len(row["checks"])
            for row in summary["records"]
            if row["provider"] == "codex" and row["arm"] == "full"
        ),
        design_source_roles=dict(Counter(cases[-1]["source_kinds"])),
    )
    summary["verification"] = dict(
        workspace=test_totals(CHECKS / "verified-workspace.log"),
        child_load=test_totals(CHECKS / "child-load.log"),
        heldout_source_reproduced=read(PRIVATE / "heldout-reproducibility.json"),
        tui=read(PRIVATE / "tui-final/receipt.json"),
    )
    summary["environment"] = read(PRIVATE / "bounded/env.json")
    summary["environment"].update(read(PUBLIC / "env.json"))
    parser_log = (CHECKS / "source-parser.log").read_text()
    match = re.search(r"(\d+) passed", parser_log)
    if not match:
        raise RuntimeError("source parser tests did not pass")
    summary["verification"]["source_parser_passed"] = int(match[1])
    summary["verification"]["clippy_log_sha256"] = hashlib.sha256(
        (CHECKS / "verified-clippy.log").read_bytes()
    ).hexdigest()
    summary["parameters"] = dict(
        packet_estimated_tokens=4000,
        evidence_max_chars=8000,
        chars_per_estimated_token=4,
        assessed_questions_per_source=8,
    )
    summary["observed_claude_models"] = sorted(
        {
            name
            for path in (PRIVATE / "bounded/raw").glob("claude-*/result.json")
            for name in read(path).get("model_usage", {})
        }
    )
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                "combined_recall": summary["combined_recall"],
                "all_sessions_by_status": summary["all_sessions_by_status"],
            }
        )
    )


if __name__ == "__main__":
    main()
