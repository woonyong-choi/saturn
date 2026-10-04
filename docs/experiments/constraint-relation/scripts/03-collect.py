"""조건마다 동일 요청 세 번을 예약한 뒤 전송하고 즉시 보존한다."""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import random
import subprocess

from candidates import MAX_BYTES, encode_request
from support import (
    PRIVATE,
    PUBLIC,
    SEED,
    append_row,
    initialize,
    judge,
    now,
    read_json,
    read_rows,
    source_manifest,
    write_json,
)
from trials import build_trials
from seal import seal_trials


def collect(cohort: str) -> None:
    initialize()
    if (PRIVATE / "stopped.json").exists():
        raise RuntimeError("collection stopped; inspect persisted reason")
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("judge key missing")
    if source_manifest() != read_json(PRIVATE / "plan.json")["source_manifest"]:
        raise RuntimeError("source changed")
    if not (PRIVATE / "run.json").exists():
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=PUBLIC, text=True
        ).strip()
        dirty = subprocess.check_output(
            ["git", "diff", "HEAD", "--", str(PUBLIC / "design.md")],
            cwd=PUBLIC,
            text=True,
        )
        if dirty:
            raise RuntimeError("design is not committed")
        write_json(PRIVATE / "run.json", {"run_id": now(), "design_commit": commit})
    trials = build_trials(cohort)
    seal_trials(cohort, trials)
    random.Random(SEED).shuffle(trials)
    saved = {r["trial_id"] for r in read_rows(PRIVATE / "jev.jsonl")}
    reserved = {r["trial_id"] for r in read_rows(PRIVATE / "calls.jsonl")}
    for index, trial in enumerate(trials):
        if trial["status"] != "ready":
            continue
        if len(encode_request(trial)) > MAX_BYTES:
            raise RuntimeError("request byte limit exceeded")
        pending = [
            r for r in (1, 2, 3) if f"{trial['trial_id']}-r{r}" not in saved | reserved
        ]
        stop = False
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            for row in executor.map(lambda r: judge.post_trial(trial, r), pending):
                row.update(
                    run_id=read_json(PRIVATE / "run.json")["run_id"],
                    condition=trial["meta"]["condition"],
                )
                append_row(PRIVATE / "jev.jsonl", row)
                stop |= row.get("http_status") in (401, 403) or row.get(
                    "model_unavailable", False
                )
        if stop:
            write_json(
                PRIVATE / "stopped.json",
                {
                    "trial_id": trial["trial_id"],
                    "reason": "authentication or model rejected",
                },
            )
            raise RuntimeError("authentication or model rejected")
        if index % 50 == 0:
            print(
                {"cohort": cohort, "trials_done": index + 1, "total": len(trials)},
                flush=True,
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("cohort", choices=("original", "extension"))
    collect(parser.parse_args().cohort)


if __name__ == "__main__":
    main()
