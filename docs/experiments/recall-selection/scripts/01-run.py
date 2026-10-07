"""고정한 복원 방식의 실제 입력과 응답을 대응 비교하고 보관한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import random
import shutil
import subprocess
import sys
import tarfile
from datetime import datetime, timezone
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-selection"
RUN = PRIVATE / os.environ.get("SATURN_SELECTION_PHASE", "primary")
OLD = ROOT / ".local/experiments/context-recall/bounded"
SOURCES = {
    "permissions": [
        (
            "gitconfig",
            "edit 모드의 개별 규칙이 없을 때 .git/config 파일 편집 판정은?",
            "ask",
        ),
        ("gitignore", "같은 조건에서 .gitignore 파일 편집 판정은?", "allow"),
        ("precedence", "개별 deny와 allow가 모두 일치하면 최종 판정은?", "deny"),
        (
            "unknown_command",
            "read-only에서 unknown 동작을 포함한 셸 명령 판정은?",
            "deny",
        ),
        ("unknown", "문서 작성자의 은행 비밀번호는?", "unknown"),
    ],
    "router-key-security": [
        ("idle", "강화 방식에서 비활성 몇 분 뒤 router 키가 잠기는가? 숫자만.", "10"),
        ("maximum", "강화 방식에서 최대 몇 시간 뒤 키가 잠기는가? 숫자만.", "12"),
        ("mode", "키체인 대체 파일이 읽히기 위해 필요한 권한은? 네 자리 숫자.", "0600"),
        ("poll", "engine이 강화 방식 잠금 조건을 몇 분마다 확인하는가? 숫자만.", "1"),
        ("unknown", "실제 사용 중인 router 키의 전체 값은?", "unknown"),
    ],
    "settings": [
        ("interval", "설정 파일 확인 주기는 몇 초인가? 숫자만.", "0.5"),
        ("retry", "router 재시도 간격은 몇 초인가? 숫자만.", "5"),
        ("deadline", "첫 실패부터 router 재시도 전체 마감은 몇 초인가? 숫자만.", "10"),
        ("logs", "engine 로그 보관 기간은 며칠인가? 숫자만.", "30"),
        ("unknown", "이 문서 배포 서버의 관리자 암호는?", "unknown"),
    ],
    "records": [
        ("extensions", "extensions 표를 추가한 스키마 버전 번호는? 숫자만.", "12"),
        ("direct", "direct_installs 표를 추가한 스키마 버전 번호는? 숫자만.", "14"),
        ("active", "sessions.last_active 열을 추가한 스키마 버전 번호는? 숫자만.", "2"),
        ("changes", "run_changes 표를 추가한 스키마 버전 번호는? 숫자만.", "11"),
        ("unknown", "이 기록 저장소의 실제 백업 파일 SHA-256은?", "unknown"),
    ],
}


def save(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def load(path: Path) -> Any:
    return json.loads(path.read_text())


def helper(filename: str) -> Any:
    spec = importlib.util.spec_from_file_location(
        filename, PUBLIC.parent / "real-context-replay/scripts" / filename
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def query(questions: list[dict[str, Any]]) -> str:
    contract = "앞 기록의 근거만으로 답하세요. 도구를 사용하지 마세요. 답이 없으면 unknown입니다. 키별 값이 문자열인 JSON 객체 하나만 답하세요.\n"
    if RUN.name == "explicit":
        keys = ", ".join(q["id"] for q in questions)
        contract += f"각 문항 ID를 JSON 키로 사용하세요. 키를 바꾸거나 추가하지 마세요. 반드시 사용할 키: {keys}.\n"
    return contract + "\n".join(f"{q['id']}: {q['question']}" for q in questions)


def export(command: list[str], output: str) -> None:
    env = dict(
        os.environ,
        SATURN_REPLAY_INPUT=str(RUN / "cases.json"),
        SATURN_RECALL_QUERIES=str(RUN / "queries.json"),
        SATURN_REPLAY_OUTPUT=str(RUN / output),
        CARGO_TARGET_DIR=str(ROOT / "target/cleanup-verification"),
    )
    with (RUN / f"{output}.log").open("x") as log:
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=log, check=True)


def prepare() -> None:
    RUN.mkdir(exist_ok=False)
    cases = load(OLD / "cases.json")
    previous_questions = load(OLD / "questions.json")
    questions = {c["id"]: previous_questions[c["cluster"]] for c in cases}
    sources = {}
    for name, items in SOURCES.items():
        source = ROOT / f"docs/design/{name}.md"
        text = source.read_text()
        sources[name] = dict(
            path=str(source.relative_to(ROOT)),
            sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
        )
        (RUN / "sources").mkdir(exist_ok=True)
        shutil.copy2(source, RUN / "sources" / source.name)
        for suffix, count in [("batch", 5), ("single", 1)]:
            case_id = f"document-{name}-{suffix}"
            row = dict(
                seq=1,
                run=1,
                session=1,
                task=1,
                input=f"{name}.md 문서 내용을 기록해 줘.",
                end="Completed",
                at_ms=0,
                event={"Text": dict(agent=1, subagent=None, text=text)},
            )
            cases.append(
                dict(
                    id=case_id,
                    cluster="document",
                    rows=[row],
                    full=f"User: {row['input']}\n\nAgent:\n{text}",
                )
            )
            questions[case_id] = [
                dict(id=key, question=prompt, expected=[answer])
                for key, prompt, answer in items[:count]
            ]
    save(RUN / "cases.json", cases)
    save(RUN / "questions.json", questions)
    save(RUN / "sources.json", sources)
    save(
        RUN / "queries.json",
        [
            dict(
                id=c["id"],
                case=c["id"],
                query=query(questions[c["id"]]),
                max_chars=8000,
            )
            for c in cases
        ],
    )
    export(
        [
            str(PRIVATE / "baseline/engine-tests"),
            "export_recalled_evidence",
            "--ignored",
        ],
        "baseline",
    )
    for name, test in [
        ("candidate", "export_recalled_evidence"),
        ("packets", "export_real_record_packets"),
    ]:
        export(
            ["cargo", "test", "-p", "saturn-engine", "--lib", test, "--", "--ignored"],
            name,
        )
    reference = load(RUN / "packets/learning-12-4000.json")["text"]
    header = reference.split("\n\n## ", 1)[0] + "\n\n"
    footer = "\n\n" + reference.rsplit("\n\n", 1)[-1]
    jobs = []
    for case in cases:
        for arm in ["full", "baseline", "candidate"]:
            context = (
                header + case["full"] + footer
                if arm == "full"
                else load(RUN / f"packets/{case['id']}-4000.json")["text"]
            )
            question = (
                query(questions[case["id"]])
                if arm == "full"
                else load(RUN / arm / f"{case['id']}.json")["input"]
            )
            jobs.append(
                dict(
                    id=case["id"],
                    case=case["id"],
                    arm=arm,
                    context=context,
                    question=question,
                )
            )
    random.Random(7110).shuffle(jobs)
    save(RUN / "calls-plan.json", jobs)
    (RUN / "raw").mkdir()
    (RUN / "work").mkdir()
    snapshot = RUN / "code"
    snapshot.mkdir()
    for relative in [
        "saturn-terminal/engine/src/handoff_recall.rs",
        "saturn-terminal/engine/src/handoff_replay.rs",
        "saturn-terminal/core/src/sessions/ranking.rs",
        "docs/experiments/recall-selection/design.md",
        "docs/experiments/recall-selection/scripts/01-run.py",
        "docs/experiments/real-context-replay/scripts/02-collect.py",
        "docs/experiments/real-context-replay/scripts/03-analyze.py",
        "docs/experiments/same-session-compaction/scripts/05-persistent.py",
    ]:
        shutil.copy2(ROOT / relative, snapshot / Path(relative).name)
    save(
        RUN / "env.json",
        dict(
            seed=7110,
            platform=platform.platform(),
            at=datetime.now(timezone.utc).isoformat(),
            head=subprocess.check_output(
                ["git", "rev-parse", "HEAD"], text=True
            ).strip(),
            versions={
                name: subprocess.check_output([name, "--version"], text=True).strip()
                for name in ["claude", "codex", "rustc"]
            },
        ),
    )
    save(
        RUN / "seal.json",
        {
            str(p.relative_to(RUN)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in RUN.rglob("*")
            if p.is_file()
        },
    )


def result_folder(job: dict[str, Any], provider: str) -> Path:
    reuse_path = RUN / "reuse.json"
    reuse = load(reuse_path) if reuse_path.exists() else {}
    phase = reuse.get(f"{job['id']}-{job['arm']}", RUN.name)
    return PRIVATE / phase / "raw" / f"{provider}-{job['id']}-{job['arm']}"


def collect() -> None:
    module = helper("02-collect.py")
    module.PRIVATE = RUN
    reuse_path = RUN / "reuse.json"
    reuse = load(reuse_path) if reuse_path.exists() else {}
    jobs = [
        job
        for job in load(RUN / "calls-plan.json")
        if f"{job['id']}-{job['arm']}" not in reuse
    ]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(module.run_provider, provider, jobs)
            for provider in ["claude", "codex"]
        ]
        for future in futures:
            future.result()


def analyze() -> dict[str, Any]:
    module = helper("03-analyze.py")
    questions = load(RUN / "questions.json")
    records = []
    for provider in ["claude", "codex"]:
        for job in load(RUN / "calls-plan.json"):
            folder = result_folder(job, provider)
            result = (
                load(folder / "result.json")
                if (folder / "result.json").exists()
                else dict(status="not_run", provider=provider)
            )
            valid = result["status"] == "ok" and result.get("tool_calls", 0) == 0
            if provider == "claude":
                valid = (
                    valid
                    and result.get("ready") == "Ready"
                    and result.get("same_session") is True
                )
                for raw in [folder / "ready.jsonl", folder / "answer.jsonl"]:
                    if raw.exists():
                        for line in raw.read_text().splitlines():
                            content = (
                                json.loads(line).get("message", {}).get("content", [])
                            )
                            if isinstance(content, list) and any(
                                item.get("type") == "tool_use" for item in content
                            ):
                                valid = False
            answers = module.parsed(result.get("text", ""))
            checks = {
                q["id"]: valid
                and module.normalize(answers.get(q["id"]))
                in [module.normalize(x) for x in q["expected"]]
                for q in questions[job["case"]]
            }
            records.append(
                dict(
                    provider=provider,
                    source_phase=folder.parents[1].name,
                    case=job["case"],
                    trial=job["id"],
                    arm=job["arm"],
                    status=result["status"],
                    valid=valid,
                    checks=checks,
                    answers=answers,
                    input_tokens=module.tokens(result),
                    elapsed_s=result.get("elapsed_s"),
                )
            )
    groups = []
    for provider in ["claude", "codex"]:
        for subset in ["all", "conversation", "document", "single"]:
            selected = [
                r
                for r in records
                if r["provider"] == provider
                and (
                    subset == "all"
                    or (
                        subset == "conversation"
                        and not r["case"].startswith("document-")
                    )
                    or (subset == "document" and r["case"].startswith("document-"))
                    or (subset == "single" and r["case"].endswith("-single"))
                )
            ]
            for arm in ["full", "baseline", "candidate"]:
                rows = [r for r in selected if r["arm"] == arm]
                tokens = sum(r["input_tokens"] for r in rows)
                baseline = sum(
                    r["input_tokens"] for r in selected if r["arm"] == "baseline"
                )
                full = sum(r["input_tokens"] for r in selected if r["arm"] == "full")
                groups.append(
                    dict(
                        provider=provider,
                        subset=subset,
                        arm=arm,
                        runs=len(rows),
                        valid_runs=sum(r["valid"] for r in rows),
                        correct=sum(sum(r["checks"].values()) for r in rows),
                        questions=sum(len(r["checks"]) for r in rows),
                        input_tokens=tokens,
                        reduction_from_baseline=1 - tokens / baseline
                        if baseline
                        else None,
                        reduction_from_full=1 - tokens / full if full else None,
                    )
                )
    regressions = []
    for row in records:
        if row["arm"] != "candidate":
            continue
        baseline = next(
            (
                r
                for r in records
                if r["provider"] == row["provider"]
                and r["trial"] == row["trial"]
                and r["arm"] == "baseline"
            ),
            None,
        )
        if baseline:
            regressions.extend(
                dict(
                    provider=row["provider"],
                    case=row["case"],
                    question=key,
                    answer=row["answers"].get(key),
                )
                for key, passed in baseline["checks"].items()
                if passed and not row["checks"][key]
            )
    summary = dict(groups=groups, regressions=regressions, records=records)
    save(PUBLIC / "results" / f"{RUN.name}.json", summary)
    return summary


def verify() -> None:
    for relative, digest in load(RUN / "seal.json").items():
        if hashlib.sha256((RUN / relative).read_bytes()).hexdigest() != digest:
            raise RuntimeError(f"sealed file changed: {relative}")
    for job in load(RUN / "calls-plan.json"):
        for provider in ["claude", "codex"]:
            folder = result_folder(job, provider)
            if not folder.exists():
                continue
            if (folder / "context.txt").read_text() != job["context"] or (
                folder / "question.txt"
            ).read_text() != job["question"]:
                raise RuntimeError("input differs from plan")
    print("sealed inputs verified")


def archive() -> None:
    paths = sorted(
        p
        for base in [PRIVATE, PUBLIC, ROOT / ".local/verification/recall-selection"]
        for p in base.rglob("*")
        if p.is_file()
        and "__pycache__" not in p.parts
        and p.name not in {"archive.json", "SHA256SUMS"}
    )
    dependencies = [
        PUBLIC.parent / "real-context-replay/scripts" / name
        for name in ["02-collect.py", "03-analyze.py", "04-verify.py"]
    ] + [
        PUBLIC.parent / "same-session-compaction/scripts/05-persistent.py",
        PUBLIC.parent / "context-recall/scripts/05-archive.py",
    ]
    paths = sorted(set(paths + dependencies))
    manifest = {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in paths
    }
    manifest_path = PUBLIC / "data/SHA256SUMS"
    manifest_path.write_text(
        "".join(f"{digest}  {name}\n" for name, digest in manifest.items())
    )
    target = (
        ROOT
        / ".local/archives"
        / f"recall-selection-{datetime.now(timezone.utc):%Y%m%dT%H%M%SZ}.tar.gz"
    )
    with tarfile.open(target, "x:gz") as bundle:
        for path in paths + [manifest_path]:
            bundle.add(path, arcname=str(path.relative_to(ROOT)))
    with tarfile.open(target) as bundle:
        for name, digest in manifest.items():
            if hashlib.sha256(bundle.extractfile(name).read()).hexdigest() != digest:
                raise RuntimeError(f"archive mismatch: {name}")
    receipt = dict(
        path=str(target.relative_to(ROOT)),
        sha256=hashlib.sha256(target.read_bytes()).hexdigest(),
        files=len(manifest),
        manifest_sha256=hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
        verified=True,
        baseline_binary="baseline/engine-tests included for macOS arm64 replay",
    )
    save(PUBLIC / "data/archive.json", receipt)
    print(json.dumps(receipt))


def main() -> None:
    os.umask(0o077)
    action = sys.argv[1]
    if action == "prepare":
        prepare()
    elif action == "collect":
        collect()
    elif action in {"process", "analyze"}:
        summary = analyze()
        print(
            json.dumps(
                dict(
                    groups=[g for g in summary["groups"] if g["subset"] == "all"],
                    regressions=summary["regressions"],
                ),
                ensure_ascii=False,
                indent=2,
            )
        )
    elif action == "verify":
        verify()
    elif action == "archive":
        archive()
    elif action == "all":
        prepare()
        collect()
        analyze()
        verify()
        archive()
    else:
        raise ValueError("unknown action")


if __name__ == "__main__":
    main()
