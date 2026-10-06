"""같은 후보에서 네 조건을 선택·실행하고 원응답을 비공개 폴더에 남긴다. 재시도하지 않는다."""

from __future__ import annotations

import concurrent.futures
import getpass
import os
import random
import subprocess

import arms
import contract
import sample
import stub
from arm_runtime import CALLER, PRIVATE, PUBLIC, ROOT, SEED, digest, jev, read, write

LIVE = CALLER == "live"
JUDGE = jev.judge if LIVE else stub.judge
LLM = jev.llm if LIVE else stub.llm


def seal() -> None:
    """설계와 실행기를 커밋한 뒤에만 실제 수집을 시작한다."""
    files = [PUBLIC / "design.md", *sorted((PUBLIC / "scripts").glob("*.py"))]
    values = {str(p.relative_to(ROOT)): digest(p) for p in files}
    path = PRIVATE / "collection-seal.json"
    if path.exists():
        if read(path)["files"] != values:
            raise RuntimeError("collection protocol changed")
        return
    for p in files:
        subprocess.run(
            ["git", "diff", "--exit-code", "HEAD", "--", str(p)],
            cwd=ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    write(path, {"commit": commit, "files": values})


def collect_case(case: dict) -> None:
    tid = case["task_id"]
    selectors = {}
    for arm in ("jev", "llm"):
        if arm == "jev":
            selectors[arm] = JUDGE(arms.jev_state(case), arms.jev_questions(case), f"select-jev-{tid}")
        else:
            selectors[arm] = LLM(arms.SELECT_MODEL, arms.llm_select_prompt(case), f"select-llm-{tid}")
    jobs = []
    for arm in contract.ARMS:
        record = read(PRIVATE / "raw" / f"select-{arm}-{tid}.json") if arm in selectors else None
        ids = arms.select(arm, case, record)["ids"]
        jobs.append((arms.EXECUTOR, arms.executor_prompt(case, ids), f"run-{arm}-{tid}"))
    random.Random(SEED + int(tid[1:4])).shuffle(jobs)
    if LIVE:
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda j: LLM(*j), jobs))
    else:
        for job in jobs:
            LLM(*job)
    print("complete " + tid, flush=True)


def main() -> None:
    os.umask(0o077)
    if LIVE:
        jev.setup()
        seal()
        if not os.environ.get("SATURN_JUDGE_KEY"):
            os.environ["SATURN_JUDGE_KEY"] = getpass.getpass("Jev key (hidden): ")
    (PRIVATE / "raw").mkdir(parents=True, exist_ok=True)
    cases = sample.build()
    sample.check_independence(cases, sample.prior_block_texts())
    target = PRIVATE / "cases.json"
    if target.exists() and read(target) != cases:
        raise RuntimeError("fixture changed")
    write(target, cases)
    for case in cases:
        collect_case(case)


if __name__ == "__main__":
    main()
