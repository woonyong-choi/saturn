"""첫 비교의 복원 입력을 그대로 두 번 더 실행한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
BASE = ROOT / ".local/experiments/context-recall/balanced"
TARGET = BASE.parent / "replication"


def main():
    os.umask(0o077)
    summary = json.loads((PUBLIC / "results/summary.json").read_text())
    groups = [g for g in summary["groups"] if g["arm"] == "recall"]
    if len(groups) != 2 or any(
        g["correct"] != 80 or g["valid_runs"] != 10 for g in groups
    ):
        raise RuntimeError(
            "both providers must pass the primary comparison before replication"
        )
    TARGET.mkdir(exist_ok=False)
    for name in ["cases.json", "queries.json", "questions.json", "env.json"]:
        shutil.copy2(BASE / name, TARGET / name)
    (TARGET / "work").mkdir()
    (TARGET / "raw").mkdir()
    shutil.copytree(BASE / "code", TARGET / "code")
    shutil.copy2(
        PUBLIC / "replication-design.md", TARGET / "code/replication-design.md"
    )
    shutil.copy2(Path(__file__), TARGET / "code/03-replicate.py")
    jobs = []
    for repeat in [2, 3]:
        for original in json.loads((BASE / "calls-plan.json").read_text()):
            if original["arm"] == "recall":
                jobs.append(
                    dict(original, id=f"{original['case']}-r{repeat}", repeat=repeat)
                )
    random.Random(7109).shuffle(jobs)
    (TARGET / "calls-plan.json").write_text(
        json.dumps(jobs, ensure_ascii=False, indent=2)
    )
    sealed = {
        str(p.relative_to(TARGET)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in TARGET.rglob("*")
        if p.is_file()
    }
    (TARGET / "collection-seal.json").write_text(json.dumps(sealed, indent=2))
    spec = importlib.util.spec_from_file_location(
        "collector", PUBLIC.parent / "real-context-replay/scripts/02-collect.py"
    )
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    helper.PRIVATE = TARGET
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        jobs_running = [
            pool.submit(helper.run_provider, provider, jobs)
            for provider in ["claude", "codex"]
        ]
        for job in jobs_running:
            job.result()


if __name__ == "__main__":
    main()
