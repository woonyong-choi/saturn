"""과제 목록, 시작 저장소 만들기, 채점, 자체 시험."""

from __future__ import annotations

import hashlib
import json
import random
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import families  # noqa: E402

ORDER = list(families.FAMILIES)
LANGS = ("ko", "mixed")
CTXS = ("short", "long")
SEED = 542001
VARIANTS = {"dev": 4, "confirm": 6}


def sha(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def cell(family: str, variant: int) -> tuple[str, str]:
    k = (variant + ORDER.index(family)) % 4
    return LANGS[k % 2], CTXS[k // 2]


def make_tasks(split: str) -> list[dict]:
    tasks = []
    for family in ORDER:
        info = families.FAMILIES[family]
        if info["split"] != split:
            continue
        for v in range(VARIANTS[split]):
            lang, ctx = cell(family, v)
            params = info["params"](v)
            spec = info["build"](params)
            prompt = spec["spec_ko" if lang == "ko" else "spec_mixed"] + (
                " 끝나면 무엇을 고쳤는지 한 줄로만 알려 줘." if lang == "ko" else " 끝나면 what you changed 를 한 줄로만 알려 줘."
            )
            files = fixture_files(family, params, spec, ctx)
            tasks.append(dict(
                task_id=f"{family}-{v}", family_id=f"fam542-{family}", split=split, family=family, variant=v, lang=lang, ctx=ctx,
                difficulty=info["difficulty"], params=params, seed=SEED + 100 * ORDER.index(family) + v, prompt=prompt,
                fixture_hash=sha(json.dumps(files, sort_keys=True) + prompt),
            ))
    return tasks


FILLER_NAMES = ["strings", "dates", "paths", "colors", "units", "slugs", "tokens", "hashes", "tables", "queues"]


def filler(seed: int) -> dict:
    rng = random.Random(seed)
    files = {"lib/__init__.py": ""}
    for name in FILLER_NAMES:
        a, b, c = rng.randint(2, 97), rng.randint(2, 97), rng.randint(100, 999)
        files[f"lib/util_{name}.py"] = (
            f'"""{name} 도우미. 이 프로젝트의 다른 코드와 관계없다."""\n\n'
            f"SCALE = {c}\n\n\ndef scale_{name}(value):\n    return (value * {a} + {b}) % SCALE\n\n\n"
            f"def describe_{name}(items):\n    return \", \".join(f\"{name}:{{item}}\" for item in items)\n\n\n"
            f"def is_valid_{name}(value):\n    return isinstance(value, int) and 0 <= value < SCALE\n"
        )
    topics = ["배포 절차", "로그 보관", "알림 규칙", "브랜치 정책", "릴리스 주기", "장애 대응", "코드 리뷰", "성능 점검"]
    notes = ["# 운영 메모\n"]
    for topic in topics:
        notes.append(f"\n## {topic}\n")
        for i in range(4):
            notes.append(f"- {topic} 항목 {i + 1}: 기준 수치는 {rng.randint(3, 90)}이고 담당은 {rng.choice(['A팀', 'B팀', 'C팀'])}이다. 예외는 문서 {rng.randint(100, 999)}번을 따른다.\n")
    files["docs/NOTES.md"] = "".join(notes)
    files["CHANGELOG.md"] = "# 변경 기록\n" + "".join(f"\n- 0.{i}.0: 내부 정리 {rng.randint(1, 50)}건\n" for i in range(1, 15))
    return files


def fixture_files(family: str, params: dict, spec: dict, ctx: str) -> dict:
    files = dict(spec["initial"])
    if ctx == "long":
        files.update(filler(sum(map(ord, family)) + params.get("seed", 0)))
    return files


def materialize(task: dict, dest: Path, solution: bool = False) -> None:
    info = families.FAMILIES[task["family"]]
    spec = info["build"](task["params"])
    files = fixture_files(task["family"], task["params"], spec, task["ctx"])
    if solution:
        files.update(spec["solution"])
    dest.mkdir(parents=True, exist_ok=True)
    for rel, text in files.items():
        path = dest / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def run_judge(task: dict, root: Path) -> dict | None:
    cmd = [sys.executable, str(HERE / "judge.py"), task["family"], json.dumps(task["params"]), str(task["seed"]), str(root)]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=90, cwd=root)
    except subprocess.TimeoutExpired:
        return None
    if out.returncode != 0:
        return None
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return None


def grade(task: dict, agent_root: Path, ref_root: Path) -> dict:
    agent = run_judge(task, agent_root)
    ref = run_judge(task, ref_root)
    if ref is None:
        raise RuntimeError("reference run failed for " + task["task_id"])
    if agent is None:
        return dict(grader="differential-v1", success=False, passed=0, total=len(ref), failed=["<run failed>"])
    failed = sorted(k for k in ref if agent.get(k) != ref[k])
    return dict(grader="differential-v1", success=not failed, passed=len(ref) - len(failed), total=len(ref), failed=failed[:10])


def selftest(base: Path) -> list[str]:
    """시작 저장소는 실패하고 기준 해법은 통과해야 한다. 어긋난 과제 id를 돌려준다."""
    problems = []
    for split in ("dev", "confirm"):
        for task in make_tasks(split):
            ref, init = base / "ref" / task["task_id"], base / "init" / task["task_id"]
            materialize(task, ref, solution=True)
            materialize(task, init)
            reference = run_judge(task, ref)
            if reference is None or any(isinstance(v, list) and v[:1] == ["harness-raise"] for v in reference.values()):
                problems.append("reference crashes " + task["task_id"])
            elif not grade(task, ref, ref)["success"]:
                problems.append("reference fails " + task["task_id"])
            if grade(task, init, ref)["success"]:
                problems.append("initial passes " + task["task_id"])
    return problems


if __name__ == "__main__":
    root = Path(sys.argv[1]).resolve()
    bad = selftest(root)
    print(json.dumps(bad, ensure_ascii=False))
    sys.exit(1 if bad else 0)
