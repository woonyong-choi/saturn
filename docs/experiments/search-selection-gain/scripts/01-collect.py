"""수집. `dev`는 개발 계열의 Jev 응답만 받고, `sealed`는 고정한 tau와 설계가 커밋된 뒤에만 시작한다. 재시도하지 않는다."""

from __future__ import annotations

import concurrent.futures
import getpass
import os
import random
import subprocess
import sys

import fixtures
import selection
from runtime import CALLER, LIMITS, MODELS, PRIVATE, PUBLIC, ROOT, SEED, WORKERS, call_cli, digest, jev_call, read, setup, write
from lock import LOCK

CODEX_PER_CELL = 2  # 확인 평가에서 Codex가 받는 계열 수(칸마다)
REPS = 3


def committed(paths: list) -> str:
    for p in paths:
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", str(p)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()


def protocol_files() -> list:
    return [PUBLIC / "design.md", PUBLIC / "run.sh", *sorted((PUBLIC / "scripts").glob("*.py"))]


def seal(name: str, extra: list) -> None:
    if CALLER == "stub":
        return
    files = protocol_files() + extra
    values = {str(p.relative_to(ROOT)): digest(p) for p in files}
    path = PRIVATE / f"seal-{name}.json"
    if path.exists():
        if read(path)["files"] != values:
            raise RuntimeError("collection protocol changed")
        return
    write(path, {"commit": committed(files), "files": values})


def codex_subset(cases: list[dict]) -> set[str]:
    seen: dict[tuple, int] = {}
    out = set()
    for c in cases:
        cell = (c["stratum"], c["lang"], c["length"])
        if seen.get(cell, 0) < CODEX_PER_CELL:
            seen[cell] = seen.get(cell, 0) + 1
            out.add(c["task_id"])
    return out


def jev_records(c: dict) -> list[dict]:
    return [
        jev_call(selection.jev_state(c), q, f"jev-{c['task_id']}-p{n}")
        for n, q in enumerate(selection.jev_pieces(c))
    ]


def run_task(c: dict, providers: list[str], tau: float) -> None:
    recs = jev_records(c)
    jobs = []
    for provider in providers:
        model = MODELS[provider]
        for rep in range(REPS):
            sel_text = ""
            record = call_cli(provider, model, selection.llm_select_prompt(c), f"{provider}-sel-{c['task_id']}-r{rep}")
            from runtime import response_text
            sel_text, _ = response_text(record)
            for arm in selection.ARMS:
                ids = selection.select(arm, c, fixtures.BUDGET_BYTES, tau, recs, sel_text)["ids"]
                jobs.append((provider, model, selection.executor_prompt(c, ids), f"{provider}-run-{arm}-{c['task_id']}-r{rep}"))
    random.Random(SEED + int(c["task_id"][1:])).shuffle(jobs)
    for j in jobs:
        call_cli(*j)


def main(mode: str) -> None:
    setup()
    if not os.environ.get("SATURN_JUDGE_KEY"):
        os.environ["SATURN_JUDGE_KEY"] = getpass.getpass("Jev key (hidden): ")
    cases = fixtures.build()
    fixtures.check(cases, prior=set())
    write(PRIVATE / "cases.json", cases) if not (PRIVATE / "cases.json").exists() else None
    if read(PRIVATE / "cases.json") != cases:
        raise RuntimeError("fixture changed")
    if mode == "dev":
        seal("dev", [])
        for c in (c for c in cases if c["split"] == "dev"):
            jev_records(c)
            print("dev " + c["task_id"], flush=True)
        return
    seal("sealed", [LOCK])
    tau = read(LOCK)["tau"]
    sealed = [c for c in cases if c["split"] == "sealed"]
    subset = codex_subset(sealed)
    todo = [(c, ["claude", "codex"] if c["task_id"] in subset else ["claude"]) for c in sealed]
    workers = WORKERS["claude"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(run_task, c, p, tau) for c, p in todo]
        for f in concurrent.futures.as_completed(futures):
            f.result()
    print("collected sealed", len(todo), "tasks", flush=True)


if __name__ == "__main__":
    main(sys.argv[1])
