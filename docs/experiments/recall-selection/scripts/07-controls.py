"""후보 반복과 같은 입력 조건으로 원문 전체와 이전 복원을 반복한다."""

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
PRIVATE = ROOT / ".local/experiments/recall-selection"


def main() -> None:
    os.umask(0o077)
    source = PRIVATE / "explicit"
    target = PRIVATE / "control-replication"
    target.mkdir(exist_ok=False)
    for name in ["cases.json", "queries.json", "questions.json", "env.json"]:
        shutil.copy2(source / name, target / name)
    environment = json.loads((target / "env.json").read_text())
    environment.update(seed=7112, source_env="explicit/env.json", repeats=[2, 3])
    (target / "env.json").write_text(json.dumps(environment, indent=2) + "\n")
    shutil.copytree(source / "code", target / "code")
    shutil.copy2(Path(__file__), target / "code/07-controls.py")
    shutil.copy2(
        PUBLIC / "control-replication-design.md",
        target / "code/control-replication-design.md",
    )
    (target / "raw").mkdir()
    (target / "work").mkdir()
    originals = json.loads((source / "calls-plan.json").read_text())
    jobs = [
        dict(job, id=f"{job['case']}-r{repeat}", repeat=repeat)
        for repeat in [2, 3]
        for job in originals
        if job["arm"] != "candidate"
    ]
    random.Random(7112).shuffle(jobs)
    (target / "calls-plan.json").write_text(
        json.dumps(jobs, ensure_ascii=False, indent=2) + "\n"
    )
    (target / "seal.json").write_text(
        json.dumps(
            {
                str(p.relative_to(target)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in target.rglob("*")
                if p.is_file()
            },
            indent=2,
        )
    )
    spec = importlib.util.spec_from_file_location(
        "collector", PUBLIC.parent / "real-context-replay/scripts/02-collect.py"
    )
    collector = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(collector)
    collector.PRIVATE = target
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(collector.run_provider, provider, jobs)
            for provider in ["claude", "codex"]
        ]
        for future in futures:
            future.result()


if __name__ == "__main__":
    main()
