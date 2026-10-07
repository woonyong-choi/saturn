"""최종 입력의 대응 판정을 확인한 뒤 새 session으로 두 번 반복한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import time
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-selection"


def load(path: Path) -> Any:
    return json.loads(path.read_text())


def main() -> None:
    os.umask(0o077)
    source = PRIVATE / os.environ.get("SATURN_SELECTION_SOURCE", "format")
    reuse_path = source / "reuse.json"
    reuse = load(reuse_path) if reuse_path.exists() else {}
    expected = [
        (
            reuse.get(f"{job['id']}-{job['arm']}", source.name),
            f"{job['id']}-{job['arm']}",
        )
        for job in load(source / "calls-plan.json")
    ]
    deadline = time.monotonic() + 7200
    while not all(
        (PRIVATE / phase / "raw" / f"{provider}-{key}" / "result.json").exists()
        for phase, key in expected
        for provider in ["claude", "codex"]
    ):
        if time.monotonic() > deadline:
            raise TimeoutError("source collection did not finish")
        time.sleep(3)
    os.environ["SATURN_SELECTION_PHASE"] = source.name
    spec = importlib.util.spec_from_file_location("study", PUBLIC / "scripts/01-run.py")
    study = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(study)
    summary = study.analyze()
    groups = [
        g for g in summary["groups"] if g["subset"] == "all" and g["arm"] == "candidate"
    ]
    if summary["regressions"] or any(
        g["correct"] != g["questions"]
        or g["valid_runs"] != g["runs"]
        or g["reduction_from_baseline"] <= 0
        for g in groups
    ):
        raise RuntimeError("final comparison failed the repetition gate")
    target = PRIVATE / "replication"
    target.mkdir(exist_ok=False)
    for name in ["cases.json", "queries.json", "questions.json", "env.json"]:
        shutil.copy2(source / name, target / name)
    environment = load(target / "env.json")
    environment.update(seed=7111, source_env=f"{source.name}/env.json", repeats=[2, 3])
    (target / "env.json").write_text(json.dumps(environment, indent=2) + "\n")
    shutil.copytree(source / "code", target / "code")
    shutil.copy2(Path(__file__), target / "code/03-repeat.py")
    shutil.copy2(PUBLIC / "format-design.md", target / "code/format-design.md")
    (target / "raw").mkdir()
    (target / "work").mkdir()
    jobs = [
        dict(job, id=f"{job['case']}-r{repeat}", repeat=repeat)
        for repeat in [2, 3]
        for job in load(source / "calls-plan.json")
        if job["arm"] == "candidate"
    ]
    random.Random(7111).shuffle(jobs)
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
    collector = study.helper("02-collect.py")
    collector.PRIVATE = target
    print("final comparison passed; collecting 72 fresh sessions", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(collector.run_provider, provider, jobs)
            for provider in ["claude", "codex"]
        ]
        for future in futures:
            future.result()


if __name__ == "__main__":
    main()
