"""봉인·호출 수·질문·비공개 원자료의 저장 경계를 확인한다."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
from collections import Counter

from protocol import DEFINITION, QUESTION
from runtime import LIMITS, PRIVATE, PUBLIC, ROOT, load_module, read, rows

COLLECT = load_module("collect_verify", PUBLIC / "scripts/03-collect.py")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


# cost: io saved experiment artifact reads and writes; basis: estimate
def main() -> None:
    key = os.environ.get("SATURN_JUDGE_KEY")
    require(bool(key), "exact key check requires process environment")
    seal = read(PRIVATE / "design-seal.json")
    env = read(PUBLIC / "env.json")
    source = subprocess.check_output(
        [
            "git",
            "show",
            env["base_commit"] + ":saturn-terminal/core/src/routers/constraint.rs",
        ],
        cwd=ROOT,
        text=True,
    )
    match = re.search(r'IS_CONSTRAINT_TEXT: &str = "(.*?)";', source, re.S)
    require(match is not None, "baseline core question missing")
    question = re.sub(r"\\\n\s*", "", match.group(1))
    require(question == QUESTION, "current question differs from baseline core")
    for filename, checksum in seal["files"].items():
        require(
            hashlib.sha256((PUBLIC / filename).read_bytes()).hexdigest() == checksum,
            "design file changed: " + filename,
        )
    for name in ("sample-seal.json", "gold-seal.json"):
        COLLECT.check_seal(name)
    cases = {c["sample_id"]: c for c in read(PRIVATE / "samples.json")}
    require(len(cases) == 400, "unexpected sample count")
    ledger = rows(PRIVATE / "calls.jsonl")
    require(
        len({r["trial_id"] for r in ledger}) == len(ledger), "duplicate reservation"
    )
    counts = Counter(r["kind"] for r in ledger)
    require(
        all(counts[k] <= limit for k, limit in LIMITS.items()), "call limit exceeded"
    )
    require(counts["jev"] == 3600, "incomplete Jev collection")
    raw = {p.stem: read(p) for p in (PRIVATE / "raw").glob("*.json")}
    require(set(raw) == {r["trial_id"] for r in ledger}, "raw and ledger mismatch")
    for trial, record in raw.items():
        require(record["trial_id"] == trial, "trial identity mismatch")
        if record["kind"] == "jev":
            expected = COLLECT.request_body(
                cases[record["sample_id"]], record["condition"]
            )
            require(record["request"] == expected, "request body mismatch")
            require(record["ts_utc"] >= seal["ts_utc"], "Jev precedes design seal")
        else:
            require(DEFINITION in record["prompt"], "definition missing")
            if not trial.startswith("audit-"):
                require(
                    record["ts_utc"] >= seal["ts_utc"], "label precedes design seal"
                )
    observations = rows(PRIVATE / "processed.jsonl")
    require(len(observations) == 4000, "unexpected observations")
    require(
        len({(r["sample_id"], r["condition"], r["repeat"]) for r in observations})
        == 4000,
        "duplicate observation",
    )
    summary = read(PUBLIC / "results/summary.json")
    require(summary["calls"]["reserved"] == dict(counts), "summary call count mismatch")
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        checksum, name = line.split("  ")
        require(
            hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() == checksum,
            "data checksum mismatch",
        )
    for root in (PUBLIC, PRIVATE):
        for path in root.rglob("*"):
            if path.is_file():
                require(
                    key.encode() not in path.read_bytes(), "secret in saved artifact"
                )
    tracked = subprocess.check_output(
        ["git", "ls-files", "docs/experiments/constraint-scope"], cwd=ROOT, text=True
    ).splitlines()
    require(
        not any("/raw/" in p or "/processed/" in p for p in tracked), "raw data tracked"
    )
    print(
        json.dumps(
            dict(
                verified=True,
                samples=len(cases),
                observations=len(observations),
                calls=dict(counts),
                exact_key_occurrences=0,
            )
        )
    )


if __name__ == "__main__":
    main()
